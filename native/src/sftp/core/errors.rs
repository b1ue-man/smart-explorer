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
        match self {
            error @ Self::Timeout => io::Error::new(io::ErrorKind::TimedOut, error.to_string()),
            error => io::Error::other(error.to_string()),
        }
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
