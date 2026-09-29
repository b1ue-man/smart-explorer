//! Typed errors for agent-protocol replies. The wire carries only an error
//! *message* (`Frame::Err(String)`), so a daemon-proxied Share or SSH backend
//! would otherwise surface every failure as `io::ErrorKind::Other`. Callers
//! such as the CLI copy planner and the tree guards decide control flow on
//! `NotFound` and `AlreadyExists`, so the well-known operating-system error
//! texts that both agent servers forward verbatim are mapped back to their
//! kinds. Anything unrecognized stays `Other` with its message intact.
use std::io;

/// Rebuild the `io::ErrorKind` a remote `io::Error` most likely carried from
/// the message the agent protocol forwarded. A server at its request limit
/// (or a peer behind the service that asked to slow down) is congestion, so
/// transfers back off instead of failing.
pub(super) fn agent_error(message: String) -> io::Error {
    if let Some((retry_after, text)) = crate::agent_proto::parse_busy(&message) {
        return crate::vfs::congestion_error(text, retry_after);
    }
    io::Error::new(kind_from_message(&message), message)
}

fn kind_from_message(message: &str) -> io::ErrorKind {
    let lower = message.to_ascii_lowercase();
    // `(os error N)` is the `std::io::Error` display suffix on every platform.
    // Only codes whose meaning is the same on Windows and Unix (or that a
    // filesystem operation can only produce on one of them) are mapped.
    match os_error_code(&lower) {
        // ENOENT / ERROR_FILE_NOT_FOUND
        Some(2) => return io::ErrorKind::NotFound,
        // ERROR_PATH_NOT_FOUND; Unix 3 (ESRCH) never comes from file access.
        Some(3) => return io::ErrorKind::NotFound,
        // ERROR_FILE_EXISTS / ERROR_ALREADY_EXISTS; no Unix file errno uses them.
        Some(80) | Some(183) => return io::ErrorKind::AlreadyExists,
        _ => {}
    }
    if lower.contains("no such file or directory")
        || lower.contains("cannot find the file specified")
        || lower.contains("cannot find the path specified")
    {
        io::ErrorKind::NotFound
    } else if lower.contains("file exists") || lower.contains("already exists") {
        io::ErrorKind::AlreadyExists
    } else if lower.contains("permission denied") || lower.contains("access is denied") {
        io::ErrorKind::PermissionDenied
    } else if lower.contains("no space left on device")
        || lower.contains("not enough space on the disk")
    {
        // ENOSPC / ERROR_DISK_FULL by their texts (their codes differ per OS):
        // a full target ends a transfer instead of failing every file.
        io::ErrorKind::StorageFull
    } else if lower.contains("disk quota exceeded") {
        io::ErrorKind::QuotaExceeded
    } else if lower.contains("read-only file system") {
        io::ErrorKind::ReadOnlyFilesystem
    } else {
        io::ErrorKind::Other
    }
}

fn os_error_code(lower: &str) -> Option<u32> {
    let start = lower.rfind("(os error ")?;
    let rest = &lower[start + "(os error ".len()..];
    let end = rest.find(')')?;
    rest[..end].trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursive_filter_task_agent_errors_recover_not_found_and_exists_kinds() {
        for message in [
            "No such file or directory (os error 2)",
            "The system cannot find the file specified. (os error 2)",
            "The system cannot find the path specified. (os error 3)",
            "Datei fehlt: No such file or directory (os error 2)",
        ] {
            assert_eq!(
                agent_error(message.into()).kind(),
                io::ErrorKind::NotFound,
                "{message}"
            );
        }
        for message in [
            "File exists (os error 17)",
            "Cannot create a file when that file already exists. (os error 183)",
            "The file exists. (os error 80)",
        ] {
            assert_eq!(
                agent_error(message.into()).kind(),
                io::ErrorKind::AlreadyExists,
                "{message}"
            );
        }
        assert_eq!(
            agent_error("Permission denied (os error 13)".into()).kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            agent_error("Access is denied. (os error 5)".into()).kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn transfer_engine_task_busy_replies_become_congestion() {
        for message in [
            "too many concurrent agent requests",
            "too many concurrent backend requests",
        ] {
            let error = agent_error(message.into());
            assert!(crate::vfs::congestion_of(&error).is_some(), "{message}");
            assert_eq!(error.to_string(), message);
        }
        let marked = crate::agent_proto::busy_message(
            Some(std::time::Duration::from_millis(1500)),
            "Gegenstelle ausgelastet",
        );
        let error = agent_error(marked);
        let congestion = crate::vfs::congestion_of(&error).expect("congestion");
        assert_eq!(
            congestion.retry_after,
            Some(std::time::Duration::from_millis(1500))
        );
        assert_eq!(error.to_string(), "Gegenstelle ausgelastet");
        assert!(
            crate::vfs::congestion_of(&agent_error("Permission denied (os error 13)".into()))
                .is_none()
        );
    }

    #[test]
    fn recursive_filter_task_unrecognized_agent_errors_stay_other_with_their_text() {
        for message in [
            "Unbekannter Export",
            "too many concurrent agent requests",
            "Input/output error (os error 5)",
            "Not a directory (os error 20)",
        ] {
            let error = agent_error(message.into());
            assert_eq!(error.kind(), io::ErrorKind::Other, "{message}");
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn transfer_engine_task_agent_errors_recover_permanent_target_kinds() {
        for (message, kind) in [
            (
                "write: No space left on device (os error 28)",
                io::ErrorKind::StorageFull,
            ),
            (
                "There is not enough space on the disk. (os error 112)",
                io::ErrorKind::StorageFull,
            ),
            (
                "Disk quota exceeded (os error 122)",
                io::ErrorKind::QuotaExceeded,
            ),
            (
                "Read-only file system (os error 30)",
                io::ErrorKind::ReadOnlyFilesystem,
            ),
        ] {
            assert_eq!(agent_error(message.into()).kind(), kind, "{message}");
        }
    }
}
