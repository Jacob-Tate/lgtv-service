//! LG webOS SSAP WebSocket client.
//!
//! Protocol:
//!  1. Connect to wss://IP:3001 (TLS, self-signed cert — validation disabled)
//!  2. Send a "register" request with the full manifest + stored client-key.
//!  3. Receive a "registered" response (instant when the key is valid).
//!  4. Send the desired command (e.g. ssap://system/turnOff).
//!  5. Receive the command response, then close.

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use native_tls::TlsConnector;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async_tls_with_config, tungstenite::Message, Connector};

// ── Registration manifest ─────────────────────────────────────────────────────
// This is the standard LG webOS registration payload used by all third-party
// clients.  The RSA-SHA256 signature is pre-computed by LG and does not change.
// Source: ColorControl project / node-lgtv2 / python-aiowebostv.

const REGISTER_MANIFEST: &str = r#"{
  "forcePairing": false,
  "pairingType": "PROMPT",
  "manifest": {
    "manifestVersion": 1,
    "appVersion": "1.1",
    "signed": {
      "created": "20140509",
      "appId": "com.lge.test",
      "vendorId": "com.lge",
      "localizedAppNames": {
        "": "LG Remote App",
        "ko-KR": "리모컨 앱",
        "zxx-XX": "ЛГ Rэмotэ AПП"
      },
      "localizedVendorNames": {
        "": "LG Electronics"
      },
      "permissions": [
        "TEST_SECURE","CONTROL_INPUT_TEXT","CONTROL_MOUSE_AND_KEYBOARD",
        "READ_INSTALLED_APPS","READ_LGE_SDX","READ_NOTIFICATIONS","SEARCH",
        "WRITE_SETTINGS","WRITE_NOTIFICATION_ALERT","CONTROL_POWER",
        "READ_CURRENT_CHANNEL","READ_RUNNING_APPS","READ_UPDATE_INFO",
        "UPDATE_FROM_REMOTE_APP","READ_LGE_TV_INPUT_EVENTS","READ_TV_CURRENT_TIME"
      ],
      "serial": "2f930e2d2cfe083771f68e4fe7bb07"
    },
    "permissions": [
      "LAUNCH","LAUNCH_WEBAPP","APP_TO_APP","CLOSE","TEST_OPEN","TEST_PROTECTED",
      "CONTROL_AUDIO","CONTROL_DISPLAY","CONTROL_INPUT_JOYSTICK",
      "CONTROL_INPUT_MEDIA_RECORDING","CONTROL_INPUT_MEDIA_PLAYBACK",
      "CONTROL_INPUT_TV","CONTROL_POWER","CONTROL_TV_SCREEN","READ_APP_STATUS",
      "READ_CURRENT_CHANNEL","READ_INPUT_DEVICE_LIST","READ_NETWORK_STATE",
      "READ_RUNNING_APPS","READ_TV_CHANNEL_LIST","WRITE_NOTIFICATION_TOAST",
      "READ_POWER_STATE","READ_COUNTRY_INFO","READ_SETTINGS"
    ],
    "signatures": [
      {
        "signatureVersion": 1,
        "signature": "eyJhbGdvcml0aG0iOiJSU0EtU0hBMjU2Iiwia2V5SWQiOiJ0ZXN0LXNpZ25pbmctY2VydCIsInNpZ25hdHVyZVZlcnNpb24iOjF9.hrVRgjCwXVvE2OOSpDZ58hR+59aFNwYDyjQgKk3auukd7pcegmE2CzPCa0bJ0ZsRAcKkCTJrWo5iDzNhMBWRyaMOv5zWSrthlf7G128qvIlpMT0YNY+n/FaOHE73uLrS/g7swl3/qH/BGFG2Hu4RlL48eb3lLKqTt2xKHdCs6Cd4RMfJPYnzgvI4BNrFUKsjkcu+WD4OO2A27Pq1n50cMchmcaXadJhGrOqH5YmHdOCj5NSHzJYrsW0HPlpuAx/ECMeIZYDh6RMqaFM2DXzdKX9NmmyqzJ3o/0lkk/N97gfVRLW5hA29yeAwaCViZNCP8iC9aO0q9fQojoa7NQnAtw=="
      }
    ]
  }
}"#;

// ── SSAP message types ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SsapEnvelope<'a> {
    #[serde(rename = "type")]
    msg_type: &'a str,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    uri: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct SsapResponse {
    #[serde(rename = "type")]
    msg_type: String,
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(rename = "returnValue")]
    return_value: Option<bool>,
    payload: Option<serde_json::Value>,
}

// ── TLS connector (cert validation disabled for TV's self-signed cert) ────────

fn insecure_tls_connector() -> Result<Connector> {
    let connector = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .context("Building TLS connector")?;
    Ok(Connector::NativeTls(connector))
}

// ── Connection helper ─────────────────────────────────────────────────────────

/// Build the registration payload, optionally including the client key.
fn build_register_payload(client_key: Option<&str>) -> Result<serde_json::Value> {
    let mut payload: serde_json::Value = serde_json::from_str(REGISTER_MANIFEST)
        .context("Parsing registration manifest")?;
    if let Some(key) = client_key {
        payload["client-key"] = serde_json::Value::String(key.to_string());
    }
    Ok(payload)
}

/// Connect to the TV and return a split (write, read) WebSocket stream.
async fn connect(ip: &str, connect_secs: u64)
    -> Result<(
        futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
            Message>,
        futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    )>
{
    let url = format!("wss://{ip}:3001");
    let connector = insecure_tls_connector()?;

    let ws_stream = timeout(
        Duration::from_secs(connect_secs),
        connect_async_tls_with_config(&url, None, false, Some(connector)),
    )
    .await
    .with_context(|| format!("Connection to {url} timed out after {connect_secs}s"))?
    .map(|(ws, _)| ws)
    .with_context(|| format!("WebSocket handshake failed for {url}"))?;

    Ok(ws_stream.split())
}

/// Wait for a "registered" response; returns the client key if the TV sends one.
async fn wait_for_registered(
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
    ack_secs: u64,
) -> Result<Option<String>> {
    timeout(Duration::from_secs(ack_secs), async {
        loop {
            match read.next().await {
                Some(Ok(Message::Text(text))) => {
                    let resp: SsapResponse = serde_json::from_str(&text)
                        .context("Parsing register response")?;
                    match resp.msg_type.as_str() {
                        "registered" => {
                            let key = resp
                                .payload
                                .as_ref()
                                .and_then(|p| p.get("client-key"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            return Ok(key);
                        }
                        "error" => bail!("TV returned error on register: {:?}", resp.payload),
                        _ => {} // "hello", prompt messages, etc. — keep waiting
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => bail!("WebSocket error: {e}"),
                None => bail!("WebSocket closed before 'registered' response"),
            }
        }
    })
    .await
    .context("Timed out waiting for 'registered' response")?
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Send the `ssap://system/turnOff` command to the TV.
pub async fn turn_off(
    ip: &str,
    client_key: &str,
    connect_secs: u64,
    ack_secs: u64,
) -> Result<()> {
    log::info!("Connecting to TV at wss://{ip}:3001");

    let (mut write, mut read) = connect(ip, connect_secs).await?;

    // Register with stored client key — TV should accept immediately.
    let register_payload = build_register_payload(Some(client_key))?;
    let register_msg = serde_json::to_string(&SsapEnvelope {
        msg_type: "register",
        id: "register_0".to_string(),
        uri: None,
        payload: Some(register_payload),
    })?;
    write
        .send(Message::Text(register_msg.into()))
        .await
        .context("Sending register request")?;

    wait_for_registered(&mut read, ack_secs).await?;
    log::info!("Registered with TV.");

    // Send turnOff command.
    let cmd_id = uuid::Uuid::new_v4().to_string();
    let cmd_msg = serde_json::to_string(&SsapEnvelope {
        msg_type: "request",
        id: cmd_id.clone(),
        uri: Some("ssap://system/turnOff"),
        payload: None,
    })?;
    write
        .send(Message::Text(cmd_msg.into()))
        .await
        .context("Sending turnOff command")?;

    // Wait for ack (TV may close socket immediately after powering off).
    let _ = timeout(Duration::from_secs(ack_secs), async {
        loop {
            match read.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(resp) = serde_json::from_str::<SsapResponse>(&text) {
                        if resp.id.as_deref() == Some(&cmd_id) {
                            if resp.return_value == Some(false) {
                                log::warn!("TV returned returnValue=false for turnOff");
                            } else {
                                log::info!("TV acknowledged turnOff command.");
                            }
                            return;
                        }
                    }
                }
                _ => return,
            }
        }
    })
    .await;

    let _ = write.close().await;
    log::info!("TV turned off.");
    Ok(())
}

/// Send a register request to pair with the TV for the first time.
/// The TV will show a prompt; the user must accept it with the remote.
/// Returns the client key granted by the TV.
pub async fn pair(ip: &str, pair_timeout_secs: u64) -> Result<String> {
    log::info!("Connecting to TV at wss://{ip}:3001 for pairing");

    let (mut write, mut read) = connect(ip, 10).await?;

    // Register WITHOUT a client key — triggers the on-screen Allow/Deny prompt.
    let register_payload = build_register_payload(None)?;
    let register_msg = serde_json::to_string(&SsapEnvelope {
        msg_type: "register",
        id: "register_0".to_string(),
        uri: None,
        payload: Some(register_payload),
    })?;
    write
        .send(Message::Text(register_msg.into()))
        .await
        .context("Sending pairing register request")?;

    println!("Waiting for you to accept the pairing request on your TV...");

    let client_key = timeout(Duration::from_secs(pair_timeout_secs), async {
        loop {
            match read.next().await {
                Some(Ok(Message::Text(text))) => {
                    let resp: SsapResponse = serde_json::from_str(&text)
                        .context("Parsing pairing response")?;
                    match resp.msg_type.as_str() {
                        "registered" => {
                            let key = resp
                                .payload
                                .as_ref()
                                .and_then(|p| p.get("client-key"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .ok_or_else(|| anyhow!("No client-key in registered response"))?;
                            return Ok::<String, anyhow::Error>(key);
                        }
                        "error" => bail!("TV rejected pairing: {:?}", resp.payload),
                        _ => {} // "hello", prompt messages — keep waiting
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => bail!("WebSocket error during pairing: {e}"),
                None => bail!("WebSocket closed before pairing completed"),
            }
        }
    })
    .await
    .context("Timed out waiting for pairing acceptance")??;

    let _ = write.close().await;
    Ok(client_key)
}
