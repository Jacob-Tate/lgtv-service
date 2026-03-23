//! Raw TCP probe to diagnose WebSocket connectivity to the LG TV.
//!
//! Sends a minimal HTTP WebSocket upgrade request over a raw TCP socket and
//! prints exactly what the TV responds with (or how it fails).  This bypasses
//! the tungstenite handshake so we can see the raw HTTP response.

use anyhow::Result;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Probe the TV at ip:port, print the raw response, and return whether
/// the TV sent a 101 Switching Protocols response.
pub fn probe_port(ip: &str, port: u16) -> bool {
    println!("\n── Probing {ip}:{port} ─────────────────────────────────");

    let addr = format!("{ip}:{port}");
    let stream = match TcpStream::connect_timeout(
        &addr.parse().unwrap(),
        Duration::from_secs(4),
    ) {
        Ok(s) => s,
        Err(e) => {
            println!("  TCP connection FAILED: {e}");
            println!("  → Port {port} is not reachable (TV off or firewall?)");
            return false;
        }
    };
    println!("  TCP connection OK");

    probe_ws(stream, ip, port)
}

fn probe_ws(mut stream: TcpStream, ip: &str, port: u16) -> bool {
    // Send a minimal WebSocket upgrade request.
    // Use a fixed Sec-WebSocket-Key so we can verify the Accept.
    let key = "dGhlIHNhbXBsZSBub25jZQ=="; // RFC 6455 example key
    let request = format!(
        "GET / HTTP/1.1\r\n\
         Host: {ip}:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Origin: http://{ip}\r\n\
         \r\n"
    );

    if let Err(e) = stream.write_all(request.as_bytes()) {
        println!("  Failed to send HTTP request: {e}");
        return false;
    }

    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let mut buf = vec![0u8; 8192];
    let n = match stream.read(&mut buf) {
        Ok(0) => {
            println!("  Server closed connection immediately (0 bytes read).");
            println!("  → The TV rejected the upgrade request without responding.");
            println!("  → This usually means a TV setting is blocking WebSocket access.");
            println!("  → See the 'TV Settings' section in the output below.");
            return false;
        }
        Ok(n) => n,
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut
               || e.kind() == std::io::ErrorKind::WouldBlock =>
        {
            println!("  Read timed out — no response after 5 s.");
            return false;
        }
        Err(e) => {
            println!("  Read error: {e}");
            return false;
        }
    };

    let response = String::from_utf8_lossy(&buf[..n]);
    println!("  Raw response ({n} bytes):");
    for line in response.lines() {
        println!("    {line}");
    }

    let switched = response.starts_with("HTTP/1.1 101");
    if switched {
        println!("  ✓ TV sent 101 Switching Protocols — WebSocket available on port {port}");
    } else if response.starts_with("HTTP/1.1 400") {
        println!("  ✗ TV returned 400 Bad Request — WebSocket rejected (unexpected)");
    } else if response.starts_with("HTTP/1.1 403") {
        println!("  ✗ TV returned 403 Forbidden — access not allowed");
        println!("  → Enable remote access in TV settings (see below)");
    } else {
        println!("  ✗ Unexpected HTTP response");
    }
    switched
}

/// Entry point for the `test` subcommand.
pub fn run(ip: &str) -> Result<()> {
    println!("Testing LG TV connectivity at {ip}");

    let ok_3000 = probe_port(ip, 3000);
    let ok_3001 = probe_port(ip, 3001);

    println!("\n── Summary ─────────────────────────────────────────────");
    println!("  Port 3000 (ws):  {}", if ok_3000 { "OK" } else { "FAILED" });
    println!("  Port 3001 (wss): {}", if ok_3001 { "OK" } else { "FAILED" });

    if !ok_3001 {
        println!("\n── TV Settings to check ────────────────────────────────");
        println!("  The TV is rejecting or not responding to WebSocket connections.");
        println!("  Try the following on your LG C4:");
        println!();
        println!("  1. Enable IP Control:");
        println!("     Settings → All Settings → General → Devices");
        println!("     → TV Management → TV On with Mobile (enable)");
        println!();
        println!("  2. Enable network standby:");
        println!("     Settings → All Settings → General → Quick Start+ (enable)");
        println!();
        println!("  3. LG Connect Apps (older webOS):");
        println!("     Settings → General → LG Connect Apps (enable)");
        println!();
        println!("  4. Check that the TV and PC are on the same network segment.");
        println!("     (TV IP: {ip} — can you ping it?)");
        println!();
        println!("  After changing settings, power-cycle the TV and try again.");
    }

    Ok(())
}
