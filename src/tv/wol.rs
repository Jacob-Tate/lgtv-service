use anyhow::{Context, Result};
use std::net::{Ipv4Addr, SocketAddr};

/// Send a Wake-on-LAN magic packet to the given MAC address.
///
/// Broadcasts to 255.255.255.255:9 (the standard WoL port).
/// This is a synchronous UDP send and completes in microseconds.
pub fn send_magic_packet(mac: &str) -> Result<()> {
    let wol = wakey::WolPacket::from_string(mac, ':')
        .with_context(|| format!("Parsing MAC address: {mac}"))?;

    let src = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0));
    let dst = SocketAddr::from((Ipv4Addr::BROADCAST, 9));

    wol.send_magic_to(src, dst)
        .with_context(|| format!("Sending WoL packet to {mac}"))?;

    log::info!("Wake-on-LAN packet sent to {mac}");
    Ok(())
}
