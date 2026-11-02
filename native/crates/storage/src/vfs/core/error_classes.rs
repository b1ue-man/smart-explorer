//! What an I/O error means for a run: the whole target refuses writes, or
//! one entry has to be left out as an omission.
use std::io;

use super::OmissionReason;
use crate::local_access::NotRegular;

/// Depth of `io::Error` wrappers searched for the original kind.
const MAX_WRAPPED: usize = 8;

/// Whether `error`, or an `io::Error` it wraps, says the target as a whole
/// refuses writes: full (`StorageFull`), over quota (`QuotaExceeded`) or
/// read-only (`ReadOnlyFilesystem`). A sync run then ends early and keeps what
/// it finished instead of failing file after file. Local errors carry these
/// kinds from the OS (ENOSPC, EDQUOT, EROFS; ERROR_DISK_FULL,
/// ERROR_DISK_QUOTA_EXCEEDED, ERROR_WRITE_PROTECT); remote backends map their
/// protocol answers to them (FTP 452/552, WebDAV 507/413, SFTP via
/// `statvfs`, Drive `storageQuotaExceeded`, SMB disk full, Share
/// `FsErrorKind::StorageFull`).
pub fn is_target_refusal(error: &io::Error) -> bool {
    first_in_chain(error, |error| {
        matches!(
            error.kind(),
            io::ErrorKind::StorageFull
                | io::ErrorKind::QuotaExceeded
                | io::ErrorKind::ReadOnlyFilesystem
        )
        .then_some(())
    })
    .is_some()
}

/// The omission an error of reading one listed entry stands for: refused
/// links and special files (`open_read_regular`), vanished entries, denied
/// access and names the platform cannot address. `None` = an ordinary
/// failure (a later run retries the action).
pub fn omission_reason(error: &io::Error) -> Option<OmissionReason> {
    first_in_chain(error, |error| {
        if let Some(refusal) = NotRegular::of(error) {
            return Some(match refusal {
                NotRegular::Link => OmissionReason::Link,
                NotRegular::Special => OmissionReason::Special,
                // A folder where a file was listed: that file is gone.
                NotRegular::Directory => OmissionReason::Vanished,
            });
        }
        match error.kind() {
            io::ErrorKind::NotFound => Some(OmissionReason::Vanished),
            io::ErrorKind::PermissionDenied => Some(OmissionReason::Unreadable),
            io::ErrorKind::InvalidFilename => Some(OmissionReason::Unrepresentable),
            _ => None,
        }
    })
}

/// The first answer of `visit` for `error` and the `io::Error`s it wraps,
/// outermost first.
fn first_in_chain<T>(
    error: &io::Error,
    mut visit: impl FnMut(&io::Error) -> Option<T>,
) -> Option<T> {
    let mut current = error;
    for _ in 0..MAX_WRAPPED {
        if let Some(found) = visit(current) {
            return Some(found);
        }
        current = current
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<io::Error>())?;
    }
    None
}
