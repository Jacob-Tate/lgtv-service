//! Persists the reason the TV was turned off so the service knows whether to
//! send a Wake-on-LAN packet when it next starts up.
//!
//! Only "shutdown" needs to be remembered across reboots: sleep/wake and
//! lock/unlock are handled by live events within the same session.

use anyhow::{Context, Result};
use std::path::PathBuf;

fn state_path() -> PathBuf {
    crate::config::config_dir().join("state.txt")
}

/// Mark that the TV was turned off due to a clean system shutdown.
/// On the next service start we will send WoL to turn it back on.
pub fn write_shutdown() -> Result<()> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Creating state directory: {}", parent.display()))?;
    }
    std::fs::write(&path, "shutdown")
        .with_context(|| format!("Writing state file: {}", path.display()))
}

/// Returns true if the previous service run ended with a clean shutdown
/// (i.e. we turned the TV off and should send WoL now).
pub fn was_shutdown() -> bool {
    match std::fs::read_to_string(state_path()) {
        Ok(s) => s.trim() == "shutdown",
        Err(_) => false,
    }
}

/// Clear the state file.  Call after acting on it so we don't WoL twice.
pub fn clear() {
    let _ = std::fs::remove_file(state_path());
}
