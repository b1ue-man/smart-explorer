use std::io;

use super::wire::{FsErrorKind, FsResponse};

/// Preserve the legacy message-only error for every failure except NotFound.
/// That single typed case lets safety-sensitive existence probes distinguish
/// absence from access, transport, and parsing failures.
pub(super) fn response(error: &io::Error) -> FsResponse {
    let kind = (error.kind() == io::ErrorKind::NotFound).then_some(FsErrorKind::NotFound);
    FsResponse::Err {
        kind,
        msg: error.to_string(),
    }
}

pub(super) fn message(message: impl Into<String>) -> FsResponse {
    FsResponse::Err {
        kind: None,
        msg: message.into(),
    }
}

pub(super) fn into_io(kind: Option<FsErrorKind>, message: String) -> io::Error {
    match kind {
        Some(FsErrorKind::NotFound) => io::Error::new(io::ErrorKind::NotFound, message),
        Some(FsErrorKind::Unknown) | None => io::Error::other(message),
    }
}

pub(super) fn exists_from_stat<T>(result: io::Result<T>) -> io::Result<bool> {
    match result {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_round_trip_becomes_absence() {
        let response = response(&io::Error::new(io::ErrorKind::NotFound, "missing"));
        let FsResponse::Err { kind, msg } = response else {
            panic!("error response expected");
        };
        let result = exists_from_stat::<()>(Err(into_io(kind, msg)));
        assert!(!result.unwrap());
    }

    #[test]
    fn non_not_found_stat_error_is_preserved() {
        let error = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let result = exists_from_stat::<()>(Err(error)).unwrap_err();
        assert_eq!(result.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(result.to_string(), "denied");
    }

    #[test]
    fn legacy_message_only_error_remains_an_error() {
        let FsResponse::Err { kind, msg } = message("transport failed") else {
            panic!("error response expected");
        };
        let result = exists_from_stat::<()>(Err(into_io(kind, msg))).unwrap_err();
        assert_eq!(result.kind(), io::ErrorKind::Other);
        assert_eq!(result.to_string(), "transport failed");
    }
}
