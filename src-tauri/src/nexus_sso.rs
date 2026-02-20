//! Nexus Mods SSO authentication via WebSocket.
//!
//! Nexus Mods provides a WebSocket-based SSO flow for third-party applications:
//! 1. Generate a unique request ID (UUID v4)
//! 2. Connect to wss://sso.nexusmods.com
//! 3. Send handshake: {"id": "<uuid>", "token": null, "protocol": 2}
//! 4. Open browser to https://www.nexusmods.com/sso?id=<uuid>&application=<name>
//! 5. Wait for WebSocket response with the user's API key
//!
//! The result is a personal API key that can be used with the Nexus Mods API.

#![allow(clippy::result_large_err)]

use std::time::{Duration, Instant};

use thiserror::Error;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const SSO_WEBSOCKET_URL: &str = "wss://sso.nexusmods.com";
const SSO_BROWSER_URL: &str = "https://www.nexusmods.com/sso";
const SSO_APP_SLUG: &str = "Corkscrew";
const SSO_TIMEOUT_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SsoError {
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tungstenite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SSO authorization timed out (5 minutes)")]
    Timeout,

    #[error("SSO failed: {0}")]
    Failed(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// UUID v4 generation (from random bytes)
// ---------------------------------------------------------------------------

fn generate_uuid() -> Result<String, SsoError> {
    let mut bytes = [0u8; 16];

    #[cfg(unix)]
    {
        use std::io::Read;
        let mut f = std::fs::File::open("/dev/urandom")?;
        f.read_exact(&mut bytes)?;
    }

    #[cfg(not(unix))]
    {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::SystemTime;
        for (i, chunk) in bytes.chunks_mut(8).enumerate() {
            let mut hasher = DefaultHasher::new();
            SystemTime::now().hash(&mut hasher);
            i.hash(&mut hasher);
            let val = hasher.finish().to_le_bytes();
            for (dst, src) in chunk.iter_mut().zip(val.iter()) {
                *dst = *src;
            }
        }
    }

    // Set UUID version 4 and variant bits
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    ))
}

// ---------------------------------------------------------------------------
// Browser launcher
// ---------------------------------------------------------------------------

fn open_browser(url: &str) -> Result<(), SsoError> {
    #[cfg(target_os = "macos")]
    let cmd = "open";

    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let cmd = "open";

    std::process::Command::new(cmd)
        .arg(url)
        .spawn()
        .map_err(SsoError::Io)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Set read timeout on the underlying TCP stream
// ---------------------------------------------------------------------------

fn set_read_timeout(
    socket: &WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    timeout: Duration,
) -> Result<(), SsoError> {
    match socket.get_ref() {
        MaybeTlsStream::NativeTls(tls_stream) => {
            tls_stream.get_ref().set_read_timeout(Some(timeout))?;
        }
        MaybeTlsStream::Plain(tcp_stream) => {
            tcp_stream.set_read_timeout(Some(timeout))?;
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run the Nexus Mods SSO flow. This blocks until the user authorizes or
/// the 5-minute timeout expires.
///
/// Returns the user's personal API key on success.
pub fn run_sso_flow() -> Result<String, SsoError> {
    let uuid = generate_uuid()?;

    // 1. Connect WebSocket to SSO server
    let (mut socket, _response) = connect(SSO_WEBSOCKET_URL)?;

    // Set a short read timeout so we can poll and check the overall timeout
    set_read_timeout(&socket, Duration::from_secs(2))?;

    // 2. Send SSO handshake
    let handshake = serde_json::json!({
        "id": uuid,
        "token": serde_json::Value::Null,
        "protocol": 2
    });
    socket.send(Message::Text(handshake.to_string()))?;

    // 3. Open browser for user to authorize
    let browser_url = format!(
        "{}?id={}&application={}",
        SSO_BROWSER_URL, uuid, SSO_APP_SLUG
    );
    open_browser(&browser_url)?;

    // 4. Wait for the API key response
    let start = Instant::now();
    let timeout = Duration::from_secs(SSO_TIMEOUT_SECS);

    loop {
        if start.elapsed() > timeout {
            let _ = socket.close(None);
            return Err(SsoError::Timeout);
        }

        match socket.read() {
            Ok(Message::Text(text)) => {
                let data: serde_json::Value = serde_json::from_str(&text)?;

                if data.get("success").and_then(|v| v.as_bool()) == Some(true) {
                    if let Some(api_key) = data
                        .get("data")
                        .and_then(|d| d.get("api_key"))
                        .and_then(|k| k.as_str())
                    {
                        let _ = socket.close(None);
                        return Ok(api_key.to_string());
                    }
                }

                // Handle explicit failure response
                if data.get("success").and_then(|v| v.as_bool()) == Some(false) {
                    let error = data
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("Unknown SSO error");
                    let _ = socket.close(None);
                    return Err(SsoError::Failed(error.to_string()));
                }
            }
            Ok(Message::Ping(data)) => {
                let _ = socket.send(Message::Pong(data));
            }
            Ok(Message::Close(_)) => {
                return Err(SsoError::Failed("SSO server closed connection".to_string()));
            }
            Ok(_) => {
                // Ignore binary, pong, etc.
            }
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Read timeout fired — loop back to check overall timeout
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed) => {
                return Err(SsoError::Failed("WebSocket connection closed".to_string()));
            }
            Err(e) => return Err(SsoError::WebSocket(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uuid_format() {
        let uuid = generate_uuid().expect("should generate UUID");
        // UUID v4 format: 8-4-4-4-12 hex chars
        assert_eq!(uuid.len(), 36, "UUID should be 36 chars: {}", uuid);

        let parts: Vec<&str> = uuid.split('-').collect();
        assert_eq!(parts.len(), 5, "UUID should have 5 groups");
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);

        // Version nibble should be 4
        assert!(
            parts[2].starts_with('4'),
            "UUID version nibble should be 4, got: {}",
            parts[2]
        );

        // Variant should be 8, 9, a, or b
        let variant_char = parts[3].chars().next().unwrap();
        assert!(
            "89ab".contains(variant_char),
            "UUID variant should be 8/9/a/b, got: {}",
            variant_char
        );
    }

    #[test]
    fn test_uuid_uniqueness() {
        let uuid1 = generate_uuid().expect("uuid1");
        let uuid2 = generate_uuid().expect("uuid2");
        assert_ne!(uuid1, uuid2, "Two UUIDs should be different");
    }
}
