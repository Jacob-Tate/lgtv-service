pub mod websocket;
pub mod wol;

use crate::config::Config;
use anyhow::Result;

/// Turn the TV off via the webOS WebSocket API.
///
/// If the TV is unreachable (already off, wrong IP, etc.) the error is logged
/// as a warning and `Ok(())` is returned — a missing turn-off on an already-off
/// TV should not crash the service.
pub async fn turn_off(config: &Config, client_key: &str) -> Result<()> {
    let result = websocket::turn_off(
        &config.tv.ip,
        client_key,
        config.timeouts.connect_secs,
        config.timeouts.ack_secs,
    )
    .await;

    if let Err(ref e) = result {
        log::warn!("Failed to turn off TV (may already be off): {e:#}");
        return Ok(());
    }

    result
}

/// Send a Wake-on-LAN magic packet to turn the TV on.
///
/// Fire-and-forget — errors are logged but not propagated.
pub fn wake_on_lan(config: &Config) {
    if let Err(e) = wol::send_magic_packet(&config.tv.mac) {
        log::warn!("Failed to send WoL packet: {e:#}");
    }
}
