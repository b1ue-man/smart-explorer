//! A failure after which the operation certainly published nothing: a packet
//! member that was refused or discarded before the peer made it visible. The
//! label travels inside `io::Error` (as `congestion.rs` does), so every layer
//! passes it on unchanged and the transfer engine may send exactly that file
//! again once. An error without the label has an unknown result and is never
//! repeated blindly.
use std::fmt;
use std::io;

/// Payload of an `io::Error` that certainly published nothing.
#[derive(Debug)]
pub struct Unpublished {
    pub message: String,
}

impl fmt::Display for Unpublished {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Unpublished {}

/// An `io::Error` of `kind` that names a failure with nothing published.
pub fn unpublished_error(kind: io::ErrorKind, message: impl Into<String>) -> io::Error {
    io::Error::new(
        kind,
        Unpublished {
            message: message.into(),
        },
    )
}

/// Whether `error` certainly published nothing.
pub fn unpublished_of(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<Unpublished>().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_task_unpublished_error_roundtrip() {
        let labeled = unpublished_error(io::ErrorKind::ConnectionReset, "Schreiben abgelehnt");
        assert_eq!(labeled.kind(), io::ErrorKind::ConnectionReset);
        assert!(unpublished_of(&labeled));
        assert_eq!(labeled.to_string(), "Schreiben abgelehnt");
        let plain = io::Error::new(io::ErrorKind::ConnectionReset, "Schreiben abgelehnt");
        assert!(!unpublished_of(&plain));
        assert!(!unpublished_of(&io::Error::from(io::ErrorKind::NotFound)));
    }
}
