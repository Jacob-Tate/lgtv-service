use anyhow::{anyhow, Result};
use std::ffi::c_void;
use std::sync::OnceLock;
use tokio::sync::mpsc;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Power::{
    RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification, HPOWERNOTIFY,
};
use windows::Win32::UI::WindowsAndMessaging::REGISTER_NOTIFICATION_FLAGS;

const PBT_APMSUSPEND: u32 = 0x0004;
const PBT_APMRESUMEAUTOMATIC: u32 = 0x0012;
const DEVICE_NOTIFY_CALLBACK: u32 = 2;

// ── Shared event type ─────────────────────────────────────────────────────────

/// All events that can drive the service's TV-control logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    /// PC going to sleep / hibernate.
    Sleep,
    /// PC waking from sleep.
    Wake,
    /// System is shutting down.  `reboot = true` means it will restart.
    Shutdown { reboot: bool },
    /// Service control handler asked us to stop.
    Stop,
}

// ── Static channel for OS power callback → async runtime ─────────────────────

static POWER_TX: OnceLock<mpsc::Sender<AppEvent>> = OnceLock::new();
static NOTIFY_HANDLE: OnceLock<HPOWERNOTIFY> = OnceLock::new();

#[repr(C)]
struct DeviceNotifySubscribeParameters {
    callback: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> u32,
    context: *mut c_void,
}
unsafe impl Send for DeviceNotifySubscribeParameters {}

/// Store the sender half of the event channel so the power callback can reach it.
pub fn init_tx(tx: mpsc::Sender<AppEvent>) -> Result<()> {
    POWER_TX.set(tx).map_err(|_| anyhow!("POWER_TX already initialised"))
}

/// Register for OS suspend/resume notifications.
pub fn register_power_notifications() -> Result<()> {
    let params = DeviceNotifySubscribeParameters {
        callback: power_callback,
        context:  std::ptr::null_mut(),
    };

    let handle = unsafe {
        RegisterSuspendResumeNotification(
            HANDLE(&params as *const _ as *mut c_void),
            REGISTER_NOTIFICATION_FLAGS(DEVICE_NOTIFY_CALLBACK),
        )
    }
    .map_err(|e| anyhow!("RegisterSuspendResumeNotification failed: {e}"))?;

    NOTIFY_HANDLE
        .set(handle)
        .map_err(|_| anyhow!("NOTIFY_HANDLE already set"))?;

    log::info!("Power notifications registered.");
    Ok(())
}

/// Unregister OS suspend/resume notifications.
pub fn unregister_power_notifications() {
    if let Some(handle) = NOTIFY_HANDLE.get() {
        unsafe {
            let _ = UnregisterSuspendResumeNotification(*handle);
        }
        log::info!("Power notifications unregistered.");
    }
}

unsafe extern "system" fn power_callback(
    _context:   *mut c_void,
    power_type: u32,
    _setting:   *mut c_void,
) -> u32 {
    let event = match power_type {
        PBT_APMSUSPEND         => AppEvent::Sleep,
        PBT_APMRESUMEAUTOMATIC => AppEvent::Wake,
        _                      => return 0,
    };
    if let Some(tx) = POWER_TX.get() {
        let _ = mpsc::Sender::<AppEvent>::try_send(tx, event);
    }
    0
}

// ── Reboot detection via Event ID 1074 ───────────────────────────────────────
//
// When any process calls ExitWindowsEx / InitiateSystemShutdownEx, Windows
// logs Event ID 1074 in the System channel.  The Data element named "param5"
// contains the action: "restart" or "shutdown" (always English, set by the OS).
// We query the most recent 1074 at shutdown time to decide whether to turn
// the TV off.

pub fn is_reboot() -> bool {
    match unsafe { query_event_1074() } {
        Ok(result) => result,
        Err(e) => {
            // If we can't tell, default to "shutdown" (safer for OLED).
            log::warn!("Could not determine shutdown type: {e:#} — treating as shutdown.");
            false
        }
    }
}

unsafe fn query_event_1074() -> Result<bool> {
    use windows::core::w;
    use windows::Win32::System::EventLog::{
        EvtClose, EvtNext, EvtQuery, EvtRender, EVT_HANDLE,
    };

    // EvtQueryChannelPath (0x1) | EvtQueryReverseDirection (0x200) = newest first
    let handle = EvtQuery(
        None,
        w!("System"),
        w!("*[System[EventID=1074]]"),
        0x201,
    )
    .map_err(|e| anyhow!("EvtQuery failed: {e}"))?;

    // EvtNext takes &mut [isize]; slice length is the requested count.
    let mut events = [0isize; 1];
    let mut returned = 0u32;

    if EvtNext(handle, &mut events, 1000, 0, &mut returned).is_err() || returned == 0 {
        drop(EvtClose(handle));
        // No Event 1074 — not a normal shutdown initiation (e.g. power button).
        return Ok(false);
    }

    let event = EVT_HANDLE(events[0]);

    // First call: find the required buffer size (returns ERROR_INSUFFICIENT_BUFFER).
    let mut buf_used = 0u32;
    let mut prop_count = 0u32;
    let _ = EvtRender(
        None,
        event,
        1u32, // EvtRenderEventXml
        0,
        None,
        &mut buf_used,
        &mut prop_count,
    );

    if buf_used == 0 {
        drop(EvtClose(event));
        drop(EvtClose(handle));
        return Err(anyhow!("EvtRender returned 0 buffer size"));
    }

    let mut buffer = vec![0u8; buf_used as usize + 2];
    EvtRender(
        None,
        event,
        1u32,
        buf_used,
        Some(buffer.as_mut_ptr() as *mut _),
        &mut buf_used,
        &mut prop_count,
    )
    .map_err(|e| anyhow!("EvtRender failed: {e}"))?;

    drop(EvtClose(event));
    drop(EvtClose(handle));

    // Buffer is UTF-16 LE.
    let words: Vec<u16> = buffer[..buf_used as usize]
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let xml = String::from_utf16_lossy(&words);

    // param5 holds the action: "restart" or "shutdown".
    // Set by Windows internals, always English.
    // Example: <Data Name="param5">restart</Data>
    Ok(xml.contains(">restart<"))
}
