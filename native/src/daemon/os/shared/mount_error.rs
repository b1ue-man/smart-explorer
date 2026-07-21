use std::io;

const PREFIX: &str = "smart-explorer-mount-error:";

/// Remove backend/root detail before an error crosses into the isolated host,
/// while retaining a small operation-relevant ErrorKind code.
pub(super) fn encode(error: io::Error) -> io::Error {
    if error.to_string().starts_with(PREFIX) {
        error
    } else {
        encoded(error.kind())
    }
}

pub(super) fn encoded(kind: io::ErrorKind) -> io::Error {
    io::Error::new(kind, format!("{PREFIX}{}", code_for(kind)))
}

/// Restore the ErrorKind that the existing agent framing otherwise flattens
/// to `Other`. The helper still receives no original error text or path.
pub(super) fn decode(error: io::Error) -> io::Error {
    let message = error.to_string();
    let Some(code) = message.strip_prefix(PREFIX) else {
        return error;
    };
    io::Error::new(
        kind_for(code),
        "the authorized mounted backend operation failed",
    )
}

fn code_for(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::ConnectionRefused => "connection_refused",
        io::ErrorKind::ConnectionReset => "connection_reset",
        io::ErrorKind::ConnectionAborted => "connection_aborted",
        io::ErrorKind::NotConnected => "not_connected",
        io::ErrorKind::BrokenPipe => "broken_pipe",
        io::ErrorKind::AlreadyExists => "already_exists",
        io::ErrorKind::WouldBlock => "would_block",
        io::ErrorKind::NotADirectory => "not_a_directory",
        io::ErrorKind::IsADirectory => "is_a_directory",
        io::ErrorKind::DirectoryNotEmpty => "directory_not_empty",
        io::ErrorKind::InvalidInput => "invalid_input",
        io::ErrorKind::InvalidData => "invalid_data",
        io::ErrorKind::TimedOut => "timed_out",
        io::ErrorKind::WriteZero => "write_zero",
        io::ErrorKind::Interrupted => "interrupted",
        io::ErrorKind::Unsupported => "unsupported",
        io::ErrorKind::UnexpectedEof => "unexpected_eof",
        io::ErrorKind::OutOfMemory => "out_of_memory",
        _ => "other",
    }
}

fn kind_for(code: &str) -> io::ErrorKind {
    match code {
        "not_found" => io::ErrorKind::NotFound,
        "permission_denied" => io::ErrorKind::PermissionDenied,
        "connection_refused" => io::ErrorKind::ConnectionRefused,
        "connection_reset" => io::ErrorKind::ConnectionReset,
        "connection_aborted" => io::ErrorKind::ConnectionAborted,
        "not_connected" => io::ErrorKind::NotConnected,
        "broken_pipe" => io::ErrorKind::BrokenPipe,
        "already_exists" => io::ErrorKind::AlreadyExists,
        "would_block" => io::ErrorKind::WouldBlock,
        "not_a_directory" => io::ErrorKind::NotADirectory,
        "is_a_directory" => io::ErrorKind::IsADirectory,
        "directory_not_empty" => io::ErrorKind::DirectoryNotEmpty,
        "invalid_input" => io::ErrorKind::InvalidInput,
        "invalid_data" => io::ErrorKind::InvalidData,
        "timed_out" => io::ErrorKind::TimedOut,
        "write_zero" => io::ErrorKind::WriteZero,
        "interrupted" => io::ErrorKind::Interrupted,
        "unsupported" => io::ErrorKind::Unsupported,
        "unexpected_eof" => io::ErrorKind::UnexpectedEof,
        "out_of_memory" => io::ErrorKind::OutOfMemory,
        _ => io::ErrorKind::Other,
    }
}
