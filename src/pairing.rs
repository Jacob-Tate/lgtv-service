//! Interactive first-run pairing flow.
//!
//! Run via: `lgtv-service pair`
//!
//! Connects to the TV, triggers the on-screen Allow/Deny prompt, then saves
//! the granted client key to disk so the service can authenticate silently on
//! subsequent connections.

use anyhow::{Context, Result};
use crate::config;

/// Pairing timeout: how long to wait for the user to accept on the TV.
const PAIR_TIMEOUT_SECS: u64 = 60;

pub async fn run() -> Result<()> {
    let cfg = config::load()
        .context("Loading config — make sure config.toml exists in %PROGRAMDATA%\\lgtv-service\\")?;

    println!("Connecting to TV at {} ...", cfg.tv.ip);
    println!("A pairing prompt will appear on your TV.");
    println!("Use your TV remote to select 'Allow'.");
    println!("(Waiting up to {PAIR_TIMEOUT_SECS} seconds)");

    let client_key = crate::tv::websocket::pair(&cfg.tv.ip, PAIR_TIMEOUT_SECS)
        .await
        .context("Pairing failed")?;

    config::save_client_key(&cfg.tv.client_key_path, &client_key)
        .context("Saving client key")?;

    println!(
        "Pairing successful! Client key saved to {}",
        cfg.tv.client_key_path.display()
    );
    println!("You can now start the service: sc start lgtv-service");

    Ok(())
}
