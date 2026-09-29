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
