use std::io;

use super::wire::{FsErrorKind, FsResponse};

/// Preserve legacy message detail while typing the cases that callers need for
/// safe control flow and fail-closed mount authorization.
pub(super) fn response(error: &io::Error) -> FsResponse {
    FsResponse::Err {
        kind: kind_of(error),
        msg: error.to_string(),
    }
}

/// The wire kind of `error`; congestion (a full host, a rate-limited backend
/// behind the export) travels as `Busy` so the client backs off instead of
/// failing.
pub(super) fn kind_of(error: &io::Error) -> Option<FsErrorKind> {
    if crate::vfs::congestion_of(error).is_some() {
        return Some(FsErrorKind::Busy);
    }
    match error.kind() {
        io::ErrorKind::NotFound => Some(FsErrorKind::NotFound),
        io::ErrorKind::PermissionDenied => Some(FsErrorKind::PermissionDenied),
        io::ErrorKind::AlreadyExists => Some(FsErrorKind::AlreadyExists),
        io::ErrorKind::Unsupported => Some(FsErrorKind::Unsupported),
        io::ErrorKind::StorageFull => Some(FsErrorKind::StorageFull),
        io::ErrorKind::QuotaExceeded => Some(FsErrorKind::QuotaExceeded),
        io::ErrorKind::ReadOnlyFilesystem => Some(FsErrorKind::ReadOnly),
        io::ErrorKind::FileTooLarge => Some(FsErrorKind::FileTooLarge),
        io::ErrorKind::InvalidFilename => Some(FsErrorKind::InvalidName),
        _ => None,
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
        Some(FsErrorKind::PermissionDenied) => {
            io::Error::new(io::ErrorKind::PermissionDenied, message)
        }
        Some(FsErrorKind::AlreadyExists) => io::Error::new(io::ErrorKind::AlreadyExists, message),
        Some(FsErrorKind::Unsupported) => io::Error::new(io::ErrorKind::Unsupported, message),
        Some(FsErrorKind::Busy) => crate::vfs::congestion_error(message, None),
        Some(FsErrorKind::StorageFull) => io::Error::new(io::ErrorKind::StorageFull, message),
        Some(FsErrorKind::QuotaExceeded) => io::Error::new(io::ErrorKind::QuotaExceeded, message),
        Some(FsErrorKind::ReadOnly) => io::Error::new(io::ErrorKind::ReadOnlyFilesystem, message),
        Some(FsErrorKind::FileTooLarge) => io::Error::new(io::ErrorKind::FileTooLarge, message),
        Some(FsErrorKind::InvalidName) => io::Error::new(io::ErrorKind::InvalidFilename, message),
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
    fn transfer_engine_task_busy_reply_becomes_congestion() {
        let busy = crate::vfs::congestion_error("Share-Host ausgelastet", None);
        let FsResponse::Err { kind, msg } = response(&busy) else {
            panic!("error response expected");
        };
        assert_eq!(kind, Some(FsErrorKind::Busy));
        let encoded = serde_json::to_string(&FsResponse::Err {
            kind,
            msg: msg.clone(),
        })
        .unwrap();
        assert!(encoded.contains("\"kind\":\"busy\""), "{encoded}");
        let decoded = into_io(kind, msg);
        let congestion = crate::vfs::congestion_of(&decoded).expect("congestion survives the wire");
        assert_eq!(congestion.message, "Share-Host ausgelastet");
        assert_eq!(decoded.to_string(), "Share-Host ausgelastet");

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum LegacyKind {
            NotFound,
            #[serde(other)]
            Unknown,
        }
        let legacy: LegacyKind = serde_json::from_str("\"busy\"").unwrap();
        assert!(matches!(legacy, LegacyKind::Unknown));
        assert!(!matches!(legacy, LegacyKind::NotFound));
        let plain = io::Error::new(io::ErrorKind::TimedOut, "langsam");
        assert_eq!(kind_of(&plain), None);
    }

    #[test]
    fn review_task_target_refusals_keep_their_kind_across_the_wire() {
        for (kind, wire) in [
            (io::ErrorKind::StorageFull, "storage_full"),
            (io::ErrorKind::QuotaExceeded, "quota_exceeded"),
            (io::ErrorKind::ReadOnlyFilesystem, "read_only"),
            (io::ErrorKind::FileTooLarge, "file_too_large"),
            (io::ErrorKind::InvalidFilename, "invalid_name"),
        ] {
            let FsResponse::Err { kind: sent, msg } = response(&io::Error::new(kind, "Ziel"))
            else {
                panic!("error response expected");
            };
            let encoded = serde_json::to_string(&FsResponse::Err {
                kind: sent,
                msg: msg.clone(),
            })
            .unwrap();
            assert!(
                encoded.contains(&format!("\"kind\":\"{wire}\"")),
                "{encoded}"
            );
            let received = into_io(sent, msg);
            assert_eq!(received.kind(), kind);
            assert_eq!(received.to_string(), "Ziel");
        }
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
