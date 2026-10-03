//! FTP reply classes without pretending a 550 is proof of absence.
use std::io;
use suppaftp::FtpError;

pub(super) fn map(error: FtpError) -> io::Error {
    if let FtpError::ConnectionError(error) = error { return error }
    let code = reply_code(&error);
    let kind = match code {
        Some(452) => io::ErrorKind::StorageFull,
        Some(552) => io::ErrorKind::QuotaExceeded,
        Some(530 | 532) => io::ErrorKind::PermissionDenied,
        Some(553) => io::ErrorKind::InvalidFilename,
        Some(500 | 502 | 504) => io::ErrorKind::Unsupported,
        Some(421 | 425 | 426) => io::ErrorKind::ConnectionAborted,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}

pub(super) fn reply_code(error: &FtpError) -> Option<u32> {
    match error {
        FtpError::UnexpectedResponse(response) => Some(response.status.code()),
        _ => None,
    }
}

pub(super) fn command_path(path: &str) -> io::Result<()> {
    if path.contains(['\r', '\n', '\0']) {
        return Err(io::Error::new(io::ErrorKind::InvalidFilename,
            "FTP path contains a control-command delimiter"));
    }
    Ok(())
}
