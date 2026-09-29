//! Preserve transport failures through the existing string error frame.
//! Unknown markers and ordinary filesystem errors retain their legacy meaning.
use std::io;

const PREFIX: &str = "SE_TRANSPORT_V1 ";

pub(crate) fn transport_error_message(error: &io::Error) -> String {
    let code = match error.kind() {
        io::ErrorKind::NotConnected => "not_connected",
        io::ErrorKind::ConnectionReset => "connection_reset",
        io::ErrorKind::ConnectionAborted => "connection_aborted",
        io::ErrorKind::ConnectionRefused => "connection_refused",
        io::ErrorKind::TimedOut => "timed_out",
        io::ErrorKind::BrokenPipe => "broken_pipe",
        io::ErrorKind::UnexpectedEof => "unexpected_eof",
        _ => return error.to_string(),
    };
    format!("{PREFIX}{code} {error}")
}

pub(crate) fn parse_transport_error(message: &str) -> Option<io::Error> {
    let (code, text) = message.strip_prefix(PREFIX)?.split_once(' ')?;
    let kind = match code {
        "not_connected" => io::ErrorKind::NotConnected,
        "connection_reset" => io::ErrorKind::ConnectionReset,
        "connection_aborted" => io::ErrorKind::ConnectionAborted,
        "connection_refused" => io::ErrorKind::ConnectionRefused,
        "timed_out" => io::ErrorKind::TimedOut,
        "broken_pipe" => io::ErrorKind::BrokenPipe,
        "unexpected_eof" => io::ErrorKind::UnexpectedEof,
        _ => return None,
    };
    Some(io::Error::new(kind, text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_open_task_transport_envelope_preserves_kind_and_context() {
        for kind in [io::ErrorKind::NotConnected, io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted, io::ErrorKind::ConnectionRefused,
            io::ErrorKind::TimedOut, io::ErrorKind::BrokenPipe, io::ErrorKind::UnexpectedEof] {
            let error = io::Error::new(kind, "connection lost: timed out\ncontext");
            let decoded = parse_transport_error(&transport_error_message(&error)).unwrap();
            assert_eq!(decoded.kind(), kind);
            assert_eq!(decoded.to_string(), error.to_string());
        }
        for text in ["legacy failure", "SE_TRANSPORT_V1 unknown reason", "SE_TRANSPORT_V1 timed_out"] {
            assert!(parse_transport_error(text).is_none());
        }
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::InvalidData,
            io::ErrorKind::Interrupted, io::ErrorKind::Other] {
            assert_eq!(transport_error_message(&io::Error::new(kind, "original")), "original");
        }
    }
}
