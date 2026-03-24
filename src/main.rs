mod config;
mod logging;
mod pairing;
mod power;
mod probe;
mod service;
mod state;
mod tv;

use anyhow::{Context, Result};
use windows_service::{
    service::{
        ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType,
    },
    service_manager::{ServiceManager, ServiceManagerAccess},
};

const SERVICE_NAME: &str = "lgtv-service";
const SERVICE_DISPLAY_NAME: &str = "LG TV Sleep/Wake Service";

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("install")   => cmd_install(),
        Some("uninstall") => cmd_uninstall(),
        Some("run")       => service::run(),
        Some("pair") => {
            logging::init_stderr();
            tokio::runtime::Runtime::new()
                .context("Creating tokio runtime")?
                .block_on(pairing::run())
        }
        Some("test") => {
            // Probe the TV directly without tungstenite to see the raw response.
            let ip = args.next().unwrap_or_else(|| {
                config::load()
                    .map(|c| c.tv.ip)
                    .unwrap_or_else(|_| "10.0.0.144".to_string())
            });
            probe::run(&ip)
        }
        Some("turn-off") => {
            logging::init_stderr();
            tokio::runtime::Runtime::new()
                .context("Creating tokio runtime")?
                .block_on(cmd_turn_off())
        }
        Some("turn-on") => {
            logging::init_stderr();
            cmd_turn_on()
        }
        Some("test-power") => {
            logging::init_stderr();
            tokio::runtime::Runtime::new()
                .context("Creating tokio runtime")?
                .block_on(cmd_test_power())
        }
        _ => {
            eprintln!(
                "Usage: {name} <command>

Commands:
  install      Register the Windows service (requires elevation)
  uninstall    Remove the Windows service (requires elevation)
  run          Start the service (used internally by the SCM)
  pair         Interactively pair with the LG TV (requires the TV to be on)
  turn-off     Turn the TV off immediately (Stream Deck / manual use)
  turn-on      Send a Wake-on-LAN packet to turn the TV on (Stream Deck / manual use)
  test [IP]    Diagnose TV connectivity (raw TCP probe, bypasses tungstenite)
  test-power   Turn the TV off, wait 30 s, then turn it back on via WoL

Setup:
  1. Create %PROGRAMDATA%\\lgtv-service\\config.toml  (see README)
  2. lgtv-service.exe install   (elevated)
  3. lgtv-service.exe test      (verify TV is reachable)
  4. lgtv-service.exe pair      (TV must be on)
  5. sc start {service}",
                name = std::env::args().next().unwrap_or_default(),
                service = SERVICE_NAME,
            );
            Ok(())
        }
    }
}

// ── install ───────────────────────────────────────────────────────────────────

fn cmd_install() -> Result<()> {
    logging::init_stderr();

    // Register the Windows Event Log source.
    logging::register().context("Registering event log source")?;

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("Opening service manager (are you running as Administrator?)")?;

    // The service executable path is the current binary with " run" appended
    // so the SCM can invoke it correctly.
    let exe_path = std::env::current_exe().context("Getting current exe path")?;

    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: SERVICE_DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe_path,
        launch_arguments: vec![std::ffi::OsString::from("run")],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };

    let _service = manager
        .create_service(&info, ServiceAccess::all())
        .context("Creating service")?;

    // Set description (optional, best-effort).
    // windows-service doesn't expose ChangeServiceConfig2 yet, so skip it.

    println!("Service '{SERVICE_NAME}' installed successfully.");
    println!("Next step: run `lgtv-service pair` to pair with your TV.");
    Ok(())
}

// ── uninstall ─────────────────────────────────────────────────────────────────

fn cmd_uninstall() -> Result<()> {
    logging::init_stderr();

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    )
    .context("Opening service manager")?;

    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::DELETE)
        .context("Opening service for deletion (are you running as Administrator?)")?;

    service.delete().context("Deleting service")?;

    // Deregister Event Log source (best-effort).
    let _ = logging::deregister();

    println!("Service '{SERVICE_NAME}' uninstalled.");
    Ok(())
}

// ── turn-off ──────────────────────────────────────────────────────────────────

async fn cmd_turn_off() -> Result<()> {
    let config = config::load().context("Loading config")?;
    let client_key = config::load_client_key(&config.tv.client_key_path)
        .context("Loading client key")?
        .context("No client key found — run `lgtv-service pair` first")?;

    println!("Turning TV off...");
    tv::turn_off(&config, &client_key).await?;
    println!("Done.");
    Ok(())
}

// ── turn-on ───────────────────────────────────────────────────────────────────

fn cmd_turn_on() -> Result<()> {
    let config = config::load().context("Loading config")?;
    println!("Sending Wake-on-LAN packet...");
    tv::wake_on_lan(&config);
    println!("Done. TV should be powering on now (~10 s boot time).");
    Ok(())
}

// ── test-power ────────────────────────────────────────────────────────────────

async fn cmd_test_power() -> Result<()> {
    let config = config::load().context("Loading config")?;
    let client_key = config::load_client_key(&config.tv.client_key_path)
        .context("Loading client key")?
        .context("No client key found — run `lgtv-service pair` first")?;

    println!("Turning TV off...");
    tv::turn_off(&config, &client_key).await?;
    println!("TV off. Waiting 30 seconds...");

    for remaining in (1..=30).rev() {
        print!("\r  {remaining}s remaining...   ");
        std::io::Write::flush(&mut std::io::stdout()).ok();
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
    println!("\rSending Wake-on-LAN packet...       ");

    tv::wake_on_lan(&config);
    println!("Done. TV should be powering on now (~10 s boot time).");
    Ok(())
}
