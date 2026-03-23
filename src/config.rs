use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level config loaded from %PROGRAMDATA%\lgtv-service\config.toml
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub tv: TvConfig,
    #[serde(default)]
    pub timeouts: TimeoutConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TvConfig {
    /// IPv4 address of the TV, e.g. "192.168.1.50"
    pub ip: String,
    /// MAC address for WoL, e.g. "A8:23:FE:01:02:03"
    pub mac: String,
    /// Path where the paired client key is stored.
    #[serde(default = "default_key_path")]
    pub client_key_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TimeoutConfig {
    /// Seconds to wait for WebSocket connection (default: 3)
    #[serde(default = "default_connect_secs")]
    pub connect_secs: u64,
    /// Seconds to wait for SSAP command acknowledgement (default: 2)
    #[serde(default = "default_ack_secs")]
    pub ack_secs: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            connect_secs: default_connect_secs(),
            ack_secs: default_ack_secs(),
        }
    }
}

fn default_connect_secs() -> u64 {
    3
}

fn default_ack_secs() -> u64 {
    2
}

fn default_key_path() -> PathBuf {
    config_dir().join("client_key.txt")
}

/// Returns %PROGRAMDATA%\lgtv-service, falling back to the current directory.
pub fn config_dir() -> PathBuf {
    if let Ok(p) = std::env::var("PROGRAMDATA") {
        PathBuf::from(p).join("lgtv-service")
    } else {
        PathBuf::from(".")
    }
}

/// Returns the default path for config.toml.
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Load config from the default location.
pub fn load() -> Result<Config> {
    load_from(&config_path())
}

/// Load config from an explicit path.
pub fn load_from(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Reading config file: {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("Parsing config file: {}", path.display()))
}

/// Read the client key from disk. Returns None if the file doesn't exist yet.
pub fn load_client_key(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("Reading client key: {}", path.display())),
    }
}

/// Persist the client key to disk.
pub fn save_client_key(path: &Path, key: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Creating directory: {}", parent.display()))?;
    }
    std::fs::write(path, key)
        .with_context(|| format!("Writing client key: {}", path.display()))
}
