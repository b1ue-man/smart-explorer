//! Error type and JSON envelopes of the Kotlin ↔ Rust protocol (api.md §1).
use serde_json::{json, Value};
use std::io;

/// A failed call: `kind` is one of the protocol error kinds, `message` is the
/// German display text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApiError {
    pub kind: &'static str,
    pub message: String,
}

impl ApiError {
    pub(crate) fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }

    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self::new("unsupported", message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }

    pub(crate) fn canceled() -> Self {
        Self::new("canceled", "Abgebrochen")
    }

    /// A connection or sign-in failure reported as text by the connect layer.
    pub(crate) fn connection(message: impl Into<String>) -> Self {
        let message = message.into();
        let lower = message.to_lowercase();
        let auth = [
            "auth",
            "passwort",
            "password",
            "anmeld",
            "permission denied (publickey",
            "login",
            "401 ",
        ]
        .iter()
        .any(|needle| lower.contains(needle));
        Self::new(if auth { "auth" } else { "network" }, message)
    }

    /// Prefixes the message with what was being done.
    pub(crate) fn context(mut self, action: &str) -> Self {
        self.message = format!("{action}: {}", self.message);
        self
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<io::Error> for ApiError {
    fn from(error: io::Error) -> Self {
        let kind = io_kind(error.kind());
        // OS errors carry English system text; lead with a German label.
        let message = match (error.raw_os_error(), io_label(error.kind())) {
            (Some(_), Some(label)) => format!("{label} ({error})"),
            _ => error.to_string(),
        };
        Self::new(kind, message)
    }
}

fn io_kind(kind: io::ErrorKind) -> &'static str {
    use io::ErrorKind as K;
    match kind {
        K::NotFound => "not_found",
        K::PermissionDenied => "permission",
        K::AlreadyExists => "exists",
        K::InvalidInput | K::InvalidData => "invalid",
        K::Unsupported => "unsupported",
        K::Interrupted => "canceled",
        K::WouldBlock => "busy",
        K::ConnectionRefused
        | K::ConnectionReset
        | K::ConnectionAborted
        | K::NotConnected
        | K::AddrNotAvailable
        | K::BrokenPipe
        | K::TimedOut
        | K::UnexpectedEof => "network",
        _ => "internal",
    }
}

fn io_label(kind: io::ErrorKind) -> Option<&'static str> {
    use io::ErrorKind as K;
    Some(match kind {
        K::NotFound => "Nicht gefunden",
        K::PermissionDenied => "Keine Berechtigung",
        K::AlreadyExists => "Existiert bereits",
        K::InvalidInput => "Ungültige Eingabe",
        K::ConnectionRefused => "Verbindung abgelehnt",
        K::ConnectionReset | K::ConnectionAborted => "Verbindung unterbrochen",
        K::TimedOut => "Zeitüberschreitung",
        _ => return None,
    })
}

/// True for errors after which a pooled remote connection is likely dead.
pub(crate) fn is_connection_loss(error: &io::Error) -> bool {
    use io::ErrorKind as K;
    if matches!(
        error.kind(),
        K::ConnectionReset
            | K::ConnectionAborted
            | K::NotConnected
            | K::BrokenPipe
            | K::UnexpectedEof
            | K::TimedOut
    ) {
        return true;
    }
    let text = error.to_string().to_lowercase();
    ["closed", "disconnect", "broken pipe", "connection reset"]
        .iter()
        .any(|needle| text.contains(needle))
}

/// `{"ok": value}` as JSON text.
pub(crate) fn ok_envelope(value: Value) -> String {
    json!({ "ok": value }).to_string()
}

/// `{"err": {"kind", "message"}}` as JSON text.
pub(crate) fn err_envelope(error: &ApiError) -> String {
    json!({ "err": { "kind": error.kind, "message": error.message } }).to_string()
}

/// The envelope for a call result.
pub(crate) fn envelope(result: Result<Value, ApiError>) -> String {
    match result {
        Ok(value) => ok_envelope(value),
        Err(error) => err_envelope(&error),
    }
}
