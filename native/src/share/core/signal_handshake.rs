use std::io;
use std::time::{Duration, Instant};

use super::core::eio;
use super::discovery_signal_types::DISCOVERY_EXCHANGE_CAPABILITY;
use super::signal_connection::SignalConnection;
use super::wire::{SrvMsg, TRACKED_DIRECT_CAPABILITY};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SignalCapabilities {
    pub(super) tracked_direct: bool,
    pub(super) discovery_exchange: bool,
}

pub(super) fn await_hello_ok(signal: &mut SignalConnection) -> io::Result<SignalCapabilities> {
    let started = Instant::now();
    loop {
        if started.elapsed() >= HELLO_TIMEOUT {
            return Err(eio("Share-Server Hello-Timeout"));
        }
        match signal.read_message() {
            Ok(Some(line)) if line.trim().is_empty() => {}
            Ok(Some(line)) => return parse_hello_ok(line.trim()),
            Ok(None) => return Err(eio("Share-Server trennte die Verbindung vor HelloOk")),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut => {}
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn parse_hello_ok(line: &str) -> io::Result<SignalCapabilities> {
    let message: SrvMsg = serde_json::from_str(line)
        .map_err(|error| eio(format!("ungueltige Share-Server-Hello-Antwort: {error}")))?;
    match message {
        SrvMsg::HelloOk { capabilities } => Ok(SignalCapabilities {
            tracked_direct: capabilities
                .iter()
                .any(|capability| capability == TRACKED_DIRECT_CAPABILITY),
            discovery_exchange: capabilities
                .iter()
                .any(|capability| capability == DISCOVERY_EXCHANGE_CAPABILITY),
        }),
        SrvMsg::Error { scope, msg } => Err(eio(format!("{scope}: {msg}"))),
        _ => Err(eio(
            "Share-Server antwortete vor HelloOk mit einer Nutzlast",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_hello_ok, SignalCapabilities};

    #[test]
    fn legacy_hello_ok_is_accepted_without_tracked_direct() {
        assert_eq!(
            parse_hello_ok(r#"{"t":"hello_ok"}"#).unwrap(),
            SignalCapabilities {
                tracked_direct: false,
                discovery_exchange: false,
            }
        );
    }

    #[test]
    fn capability_must_be_confirmed_by_server() {
        assert_eq!(
            parse_hello_ok(r#"{"t":"hello_ok","capabilities":["future","tracked_direct_v1"]}"#)
                .unwrap(),
            SignalCapabilities {
                tracked_direct: true,
                discovery_exchange: false,
            }
        );
    }

    #[test]
    fn discovery_capability_must_be_confirmed_by_server() {
        assert_eq!(
            parse_hello_ok(
                r#"{"t":"hello_ok","capabilities":["discovery_exchange_v1"]}"#
            )
            .unwrap(),
            SignalCapabilities {
                tracked_direct: false,
                discovery_exchange: true,
            }
        );
    }

    #[test]
    fn pre_hello_payload_is_rejected() {
        assert!(parse_hello_ok(r#"{"t":"pong"}"#).is_err());
    }
}
