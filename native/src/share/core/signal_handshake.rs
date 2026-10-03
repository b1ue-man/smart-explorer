use std::io;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::core::{eio, hex, hex_decode};
use super::discovery_signal_types::DISCOVERY_EXCHANGE_CAPABILITY;
use super::identity::ShareIdentity;
use super::signal_connection::{send_line, SignalConnection};
use super::wire::{SrvMsg, IDLE_KEEPALIVE_CAPABILITY, TRACKED_DIRECT_CAPABILITY};

/// Hello, the key-login challenge and its answer, and `hello_ok` together.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// The device proves its Iroh key to the server (FC4); servers that do not
/// know it answer `hello_ok` at once and the session stays unproven.
pub(super) const KEY_LOGIN_CAPABILITY: &str = "key_login_v1";
/// Domain of the signed login digest; the same key signs other protocols.
const LOGIN_DOMAIN: &[u8] = b"se-signal-login-v1\0";
const LOGIN_NONCE_BYTES: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SignalCapabilities {
    pub(super) tracked_direct: bool,
    pub(super) discovery_exchange: bool,
    pub(super) idle_keepalive: bool,
    /// The server verified this device's key; it binds device id, own
    /// lookup and room entries to the key and accepts access proofs.
    pub(super) key_login: bool,
}

#[derive(Deserialize)]
#[serde(tag = "t", rename = "hello_challenge")]
struct HelloChallenge {
    nonce: String,
}

#[derive(Serialize)]
#[serde(tag = "t", rename = "hello_auth")]
struct HelloAuth {
    signature: String,
}

/// Waits for `hello_ok`, answering a key-login challenge on the way.
pub(super) fn await_hello_ok(
    signal: &mut SignalConnection,
    identity: &ShareIdentity,
) -> io::Result<SignalCapabilities> {
    let started = Instant::now();
    let mut answered = false;
    loop {
        if started.elapsed() >= HELLO_TIMEOUT {
            return Err(eio("Share-Server Hello-Timeout"));
        }
        let line = match signal.read_message() {
            Ok(Some(line)) => line,
            Ok(None) => return Err(eio("Share-Server trennte die Verbindung vor HelloOk")),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                continue
            }
            Err(error) => return Err(error),
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(nonce) = parse_challenge(line)? {
            if answered {
                return Err(eio(
                    "Share-Server verlangte die Schluessel-Anmeldung doppelt",
                ));
            }
            let signature = login_signature(&nonce, identity);
            send_line(signal, &HelloAuth { signature })?;
            answered = true;
            continue;
        }
        let capabilities = parse_hello_ok(line)?;
        if capabilities.key_login != answered {
            return Err(eio(
                "Share-Server meldete eine Schluessel-Anmeldung, die nicht stattfand",
            ));
        }
        return Ok(capabilities);
    }
}

/// The nonce of a `hello_challenge`; `None` for every other message.
fn parse_challenge(line: &str) -> io::Result<Option<Vec<u8>>> {
    let value: serde_json::Value = serde_json::from_str(line)
        .map_err(|error| eio(format!("ungueltige Share-Server-Hello-Antwort: {error}")))?;
    if value.get("t").and_then(serde_json::Value::as_str) != Some("hello_challenge") {
        return Ok(None);
    }
    let challenge: HelloChallenge = serde_json::from_value(value)
        .map_err(|error| eio(format!("ungueltige Anmelde-Aufforderung: {error}")))?;
    let nonce = hex_decode(&challenge.nonce)
        .ok()
        .filter(|nonce| nonce.len() == LOGIN_NONCE_BYTES)
        .ok_or_else(|| eio("Anmelde-Aufforderung des Share-Servers ist ungueltig"))?;
    Ok(Some(nonce))
}

/// Signs the login digest with the device's Iroh key (hex, 64 bytes).
fn login_signature(nonce: &[u8], identity: &ShareIdentity) -> String {
    let digest = login_digest(nonce, &identity.device_id, &identity.public_key);
    hex(&identity.iroh_secret.sign(&digest).to_bytes())
}

/// SHA-256 over the domain and the length-prefixed nonce, device id and
/// public key of the Hello; `se-share-server` computes the same.
pub(super) fn login_digest(nonce: &[u8], device_id: &str, public_key: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(LOGIN_DOMAIN);
    for field in [nonce, device_id.as_bytes(), public_key.as_bytes()] {
        let length = u32::try_from(field.len()).unwrap_or(u32::MAX);
        hasher.update(length.to_be_bytes());
        hasher.update(field);
    }
    hasher.finalize().into()
}

pub(super) fn parse_hello_ok(line: &str) -> io::Result<SignalCapabilities> {
    let message: SrvMsg = serde_json::from_str(line)
        .map_err(|error| eio(format!("ungueltige Share-Server-Hello-Antwort: {error}")))?;
    match message {
        SrvMsg::HelloOk { capabilities } => {
            let has = |wanted: &str| capabilities.iter().any(|capability| capability == wanted);
            Ok(SignalCapabilities {
                tracked_direct: has(TRACKED_DIRECT_CAPABILITY),
                discovery_exchange: has(DISCOVERY_EXCHANGE_CAPABILITY),
                idle_keepalive: has(IDLE_KEEPALIVE_CAPABILITY),
                key_login: has(KEY_LOGIN_CAPABILITY),
            })
        }
        SrvMsg::Error { scope, msg } => Err(eio(format!("{scope}: {msg}"))),
        _ => Err(eio(
            "Share-Server antwortete vor HelloOk mit einer Nutzlast",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{login_digest, parse_challenge, parse_hello_ok, SignalCapabilities};

    #[test]
    fn legacy_hello_ok_is_accepted_without_tracked_direct() {
        assert_eq!(
            parse_hello_ok(r#"{"t":"hello_ok"}"#).unwrap(),
            SignalCapabilities::default()
        );
    }

    #[test]
    fn capability_must_be_confirmed_by_server() {
        assert_eq!(
            parse_hello_ok(r#"{"t":"hello_ok","capabilities":["future","tracked_direct_v1"]}"#)
                .unwrap(),
            SignalCapabilities {
                tracked_direct: true,
                ..SignalCapabilities::default()
            }
        );
    }

    #[test]
    fn discovery_capability_must_be_confirmed_by_server() {
        assert_eq!(
            parse_hello_ok(r#"{"t":"hello_ok","capabilities":["discovery_exchange_v1"]}"#).unwrap(),
            SignalCapabilities {
                discovery_exchange: true,
                ..SignalCapabilities::default()
            }
        );
    }

    #[test]
    fn android_background_task_idle_capability_must_be_confirmed_by_server() {
        assert_eq!(
            parse_hello_ok(r#"{"t":"hello_ok","capabilities":["idle_keepalive_v1"]}"#).unwrap(),
            SignalCapabilities {
                idle_keepalive: true,
                ..SignalCapabilities::default()
            }
        );
        assert!(
            !parse_hello_ok(r#"{"t":"hello_ok","capabilities":["idle_keepalive_v2"]}"#)
                .unwrap()
                .idle_keepalive
        );
    }

    #[test]
    fn pre_hello_payload_is_rejected() {
        assert!(parse_hello_ok(r#"{"t":"pong"}"#).is_err());
    }

    #[test]
    fn review_task_key_login_is_reported_only_when_confirmed() {
        assert!(
            parse_hello_ok(r#"{"t":"hello_ok","capabilities":["key_login_v1"]}"#)
                .unwrap()
                .key_login
        );
        let challenge = r#"{"t":"hello_challenge","nonce":"000102030405060708090a0b0c0d0e0f"}"#;
        assert_eq!(
            parse_challenge(challenge).unwrap(),
            Some((0u8..16).collect::<Vec<_>>())
        );
        assert_eq!(parse_challenge(r#"{"t":"hello_ok"}"#).unwrap(), None);
        assert!(parse_challenge(r#"{"t":"hello_challenge","nonce":"00"}"#).is_err());
    }

    /// Shared test vector with `se-share-server` (`login.rs`): both sides
    /// must derive the same digest from the same Hello and nonce.
    #[test]
    fn review_task_login_digest_matches_the_server_vector() {
        let nonce: Vec<u8> = (0u8..16).collect();
        let digest = login_digest(&nonce, "device-a", "pk-a");
        assert_eq!(crate::share::core::hex(&digest), LOGIN_DIGEST_VECTOR);
    }

    const LOGIN_DIGEST_VECTOR: &str =
        "7cfb3e8edf96d9906d5c895e201146e639b1a74b2ab473401db0614c85e09bac";
}
