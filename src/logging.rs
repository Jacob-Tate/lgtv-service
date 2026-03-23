use anyhow::Result;

const SOURCE: &str = "lgtv-service";

/// Register the Windows Event Log source (call once during `install`).
pub fn register() -> Result<()> {
    eventlog::register(SOURCE).map_err(|e| anyhow::anyhow!("Event log register: {e}"))?;
    Ok(())
}

/// Deregister the Windows Event Log source (call during `uninstall`).
pub fn deregister() -> Result<()> {
    eventlog::deregister(SOURCE).map_err(|e| anyhow::anyhow!("Event log deregister: {e}"))?;
    Ok(())
}

/// Initialize the log facade to write to the Windows Event Log.
/// Call at service startup (inside the service main, after the runtime is ready).
pub fn init_service() {
    if let Err(e) = eventlog::init(SOURCE, log::Level::Info) {
        // Fallback: if event log init fails, swallow — the service still runs.
        eprintln!("Warning: failed to initialize event log: {e}");
    }
}

/// Initialize a stderr logger for interactive subcommands (pair, install, etc.).
pub fn init_stderr() {
    let _ = env_logger_init();
}

fn env_logger_init() -> Result<(), log::SetLoggerError> {
    // Simple stderr logger with INFO default.
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init()
}
