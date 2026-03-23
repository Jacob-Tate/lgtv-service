//! Windows service entry point and orchestration.
//!
//! Thread model:
//!  - The SCM calls `ffi_service_main` on its own thread.
//!  - We create a tokio Runtime and block on `async_service_main`.
//!  - The OS power callback (kernel thread) sends AppEvents via a static mpsc sender.
//!  - The SCM control handler (SCM thread) sends AppEvents via a cloned mpsc sender.
//!  - The async loop drives all TV actions.

use anyhow::{Context, Result};
use std::ffi::OsString;
use std::time::Duration;
use tokio::sync::mpsc;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

use crate::power::AppEvent;

const SERVICE_NAME: &str = "lgtv-service";

define_windows_service!(ffi_service_main, service_main);

pub fn run() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("Starting service dispatcher")?;
    Ok(())
}

// ── Service main ──────────────────────────────────────────────────────────────

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run_service() {
        log::error!("Service exited with error: {e:#}");
    }
}

fn run_service() -> Result<()> {
    // Single channel for all events: power callback + SCM control handler.
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>(16);

    // Clone for the SCM control handler closure (mpsc::Sender is Clone + Send).
    let event_tx_scm = event_tx.clone();

    // --- Register SCM control handler ----------------------------------------
    let status_handle = service_control_handler::register(
        SERVICE_NAME,
        move |control| match control {
            ServiceControl::Stop => {
                let _ = event_tx_scm.try_send(AppEvent::Stop);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Shutdown => {
                // Query Event 1074 to determine if this is a reboot or shutdown.
                let reboot = crate::power::is_reboot();
                log::info!(
                    "System {} detected.",
                    if reboot { "reboot" } else { "shutdown" }
                );
                let _ = event_tx_scm.try_send(AppEvent::Shutdown { reboot });
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::SessionChange(_) => ServiceControlHandlerResult::NoError,
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _                           => ServiceControlHandlerResult::NotImplemented,
        },
    )
    .context("Registering service control handler")?;

    // Report StartPending.
    status_handle
        .set_service_status(ServiceStatus {
            service_type:      ServiceType::OWN_PROCESS,
            current_state:     ServiceState::StartPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code:         ServiceExitCode::Win32(0),
            checkpoint:        0,
            wait_hint:         Duration::from_secs(5),
            process_id:        None,
        })
        .context("Reporting StartPending")?;

    // --- Build tokio runtime -------------------------------------------------
    let runtime = tokio::runtime::Runtime::new().context("Creating tokio runtime")?;

    // Report Running.
    status_handle
        .set_service_status(ServiceStatus {
            service_type:      ServiceType::OWN_PROCESS,
            current_state:     ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP
                | ServiceControlAccept::SHUTDOWN
                | ServiceControlAccept::SESSION_CHANGE,
            exit_code:         ServiceExitCode::Win32(0),
            checkpoint:        0,
            wait_hint:         Duration::ZERO,
            process_id:        None,
        })
        .context("Reporting Running")?;

    // Initialise the static power-callback sender BEFORE registering power
    // notifications so the callback always has a valid sender.
    crate::power::init_tx(event_tx).context("Initialising power event sender")?;

    // --- Run async main (blocks until service exits) -------------------------
    runtime.block_on(async_service_main(event_rx))?;

    // Report Stopped.
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type:      ServiceType::OWN_PROCESS,
        current_state:     ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code:         ServiceExitCode::Win32(0),
        checkpoint:        0,
        wait_hint:         Duration::ZERO,
        process_id:        None,
    });

    Ok(())
}

// ── Async service body ────────────────────────────────────────────────────────

async fn async_service_main(mut event_rx: mpsc::Receiver<AppEvent>) -> Result<()> {
    crate::logging::init_service();
    log::info!("lgtv-service starting.");

    let config = crate::config::load().context("Loading config")?;
    let client_key = crate::config::load_client_key(&config.tv.client_key_path)
        .context("Loading client key")?
        .unwrap_or_default();

    if client_key.is_empty() {
        log::warn!(
            "No client key found at {}. Run `lgtv-service pair` first.",
            config.tv.client_key_path.display()
        );
    }

    // Register OS power notifications (sleep/wake).
    crate::power::register_power_notifications().context("Registering power notifications")?;

    // On startup after a clean shutdown: the TV was turned off — wake it.
    if crate::state::was_shutdown() {
        log::info!("Previous shutdown detected — sending WoL to turn TV on.");
        crate::state::clear();
        crate::tv::wake_on_lan(&config);
    }

    log::info!("lgtv-service running. Watching for sleep/wake/lock/unlock/shutdown events.");

    // ── Main event loop ───────────────────────────────────────────────────────
    while let Some(event) = event_rx.recv().await {
        match event {
            AppEvent::Sleep => {
                log::info!("Sleep — turning TV off.");
                if let Err(e) = crate::tv::turn_off(&config, &client_key).await {
                    log::warn!("turn_off: {e:#}");
                }
            }

            AppEvent::Wake => {
                log::info!("Wake — waiting for NIC, then sending WoL (3 attempts).");
                for attempt in 1..=3u32 {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    log::info!("WoL attempt {attempt}/3");
                    crate::tv::wake_on_lan(&config);
                }
            }

AppEvent::Shutdown { reboot: false } => {
                log::info!("Shutdown — turning TV off.");
                if let Err(e) = crate::tv::turn_off(&config, &client_key).await {
                    log::warn!("turn_off: {e:#}");
                }
                if let Err(e) = crate::state::write_shutdown() {
                    log::warn!("Could not write shutdown state: {e:#}");
                }
                break;
            }

            AppEvent::Shutdown { reboot: true } => {
                log::info!("Reboot — leaving TV on.");
                // Don't write state: no WoL needed on next service start.
                break;
            }

            AppEvent::Stop => {
                log::info!("Stop requested.");
                break;
            }
        }
    }

    crate::power::unregister_power_notifications();
    log::info!("lgtv-service stopped.");
    Ok(())
}
