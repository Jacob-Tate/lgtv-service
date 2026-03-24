use anyhow::Result;
use simplelog::{Config, LevelFilter, WriteLogger};
use std::fs::OpenOptions;

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

/// Returns the path to the service log file:
/// `%PROGRAMDATA%\lgtv-service\lgtv-service.log`
pub fn log_file_path() -> std::path::PathBuf {
    let base = std::env::var("PROGRAMDATA").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base)
        .join("lgtv-service")
        .join("lgtv-service.log")
}

/// Initialize file-based logging for the Windows service.
/// Appends to `%PROGRAMDATA%\lgtv-service\lgtv-service.log` so logs
/// survive across restarts.  Falls back to the Windows Event Log if the
/// file cannot be opened.
pub fn init_service() {
    let log_path = log_file_path();

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(file) => {
            if let Err(e) = WriteLogger::init(LevelFilter::Info, Config::default(), file) {
                eprintln!("Warning: failed to initialize file logger: {e}");
                init_event_log_fallback();
            }
        }
        Err(e) => {
            eprintln!(
                "Warning: could not open log file {}: {e} — falling back to Event Log",
                log_path.display()
            );
            init_event_log_fallback();
        }
    }
}

fn init_event_log_fallback() {
    if let Err(e) = eventlog::init(SOURCE, log::Level::Info) {
        eprintln!("Warning: failed to initialize event log: {e}");
    }
}

/// Initialize a stderr logger for interactive subcommands (pair, install, etc.).
pub fn init_stderr() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();
}
