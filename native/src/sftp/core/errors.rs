use std::io;

pub(crate) trait IntoIoError {
    fn into_io_error(self) -> io::Error;
}

pub(crate) fn io_err(error: impl IntoIoError) -> io::Error {
    error.into_io_error()
}

impl IntoIoError for io::Error {
    fn into_io_error(self) -> io::Error {
        self
    }
}

impl IntoIoError for russh::Error {
    fn into_io_error(self) -> io::Error {
        match self {
            Self::IO(error) => error,
            error @ (Self::ConnectionTimeout
            | Self::KeepaliveTimeout
            | Self::InactivityTimeout
            | Self::Elapsed(_)) => io::Error::new(io::ErrorKind::TimedOut, error.to_string()),
            error => io::Error::other(error.to_string()),
        }
    }
}

impl IntoIoError for russh::keys::Error {
    fn into_io_error(self) -> io::Error {
        match self {
            Self::IO(error) => error,
            error => io::Error::other(error.to_string()),
        }
    }
}

impl IntoIoError for russh_sftp::client::error::Error {
    fn into_io_error(self) -> io::Error {
        use russh_sftp::protocol::StatusCode;
        let kind = match &self {
            Self::Timeout => io::ErrorKind::TimedOut,
            Self::Status(status) => match status.status_code {
                StatusCode::NoSuchFile => io::ErrorKind::NotFound,
                StatusCode::PermissionDenied => io::ErrorKind::PermissionDenied,
                StatusCode::OpUnsupported => io::ErrorKind::Unsupported,
                StatusCode::NoConnection | StatusCode::ConnectionLost => {
                    io::ErrorKind::ConnectionAborted
                }
                StatusCode::BadMessage => io::ErrorKind::InvalidInput,
                _ => io::ErrorKind::Other,
            },
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, self.to_string())
    }
}

impl IntoIoError for String {
    fn into_io_error(self) -> io::Error {
        io::Error::other(self)
    }
}

impl IntoIoError for &str {
    fn into_io_error(self) -> io::Error {
        io::Error::other(self.to_string())
    }
}
