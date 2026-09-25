//! Rename that never replaces an existing name, with a fallback chain for
//! Android storage.
//!
//! 1. `renameat2(RENAME_NOREPLACE)`: atomic. Raw syscall as on Linux; bionic
//!    lists `renameat2` in `SYSCALLS.TXT` since API 30 (the minimum API
//!    level), so the app seccomp allowlist generated from it permits the call.
//! 2. The kernel or filesystem lacks the flag (`EINVAL`, `ENOSYS`,
//!    `EOPNOTSUPP`; a FUSE server without rename2 support, such as shared
//!    storage may be, yields `EINVAL`) and the source is not a directory: hard
//!    link to the destination, which is create-only and therefore still
//!    refuses an existing name atomically, then remove the source.
//! 3. The link failed for another reason than an existing name (FUSE storage
//!    may not offer hard links), or the source is a directory: existence
//!    check, then `rename`. Documented residual risk: a name created between
//!    the check and the rename is replaced (a file or an empty directory); a
//!    non-empty directory is never replaced (`ENOTEMPTY`).
//!
//! Each error number that triggered a fallback is recorded once per process
//! and, once host values are set (the Android app), appended to
//! `<app data>/android-fs.log` as evidence of what the device storage supports.
use std::ffi::CString;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Mutex;

const LOG_FILE: &str = "android-fs.log";

static OBSERVED: Mutex<Vec<(Attempt, i32)>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attempt {
    Renameat2,
    HardLink,
}

impl Attempt {
    fn label(self) -> &'static str {
        match self {
            Attempt::Renameat2 => "renameat2(RENAME_NOREPLACE)",
            Attempt::HardLink => "link",
        }
    }
}

/// The fallback step the chain takes next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Report the error: the operation itself was refused.
    Fail,
    /// Create-only hard link, then remove the source name.
    HardLink,
    /// Existence check, then plain `rename`.
    CheckedRename,
}

pub(crate) fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let error = match renameat2_no_replace(source, destination) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if !flag_unsupported(error.raw_os_error()) {
        return Err(error);
    }
    record_fallback(Attempt::Renameat2, error.raw_os_error());
    let source_is_dir = std::fs::symlink_metadata(source)?.is_dir();
    if fallback_for(source_is_dir) == Step::HardLink {
        match std::fs::hard_link(source, destination) {
            Ok(()) => return remove_linked_source(source, destination),
            Err(error) if after_link_failure(error.raw_os_error()) == Step::Fail => {
                return Err(error)
            }
            Err(error) => record_fallback(Attempt::HardLink, error.raw_os_error()),
        }
    }
    checked_rename(source, destination)
}

/// `renameat2` failed because the no-replace flag is unsupported, not because
/// the rename itself was refused.
pub(super) fn flag_unsupported(errno: Option<i32>) -> bool {
    matches!(
        errno,
        Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EOPNOTSUPP)
    )
}

/// First fallback once the flag is unsupported: directories cannot be linked.
pub(super) fn fallback_for(source_is_dir: bool) -> Step {
    if source_is_dir {
        Step::CheckedRename
    } else {
        Step::HardLink
    }
}

/// After the create-only link failed: an existing destination is the refusal
/// this primitive exists for; anything else means links are unavailable, and
/// the rename reports a genuine failure itself.
pub(super) fn after_link_failure(errno: Option<i32>) -> Step {
    if errno == Some(libc::EEXIST) {
        Step::Fail
    } else {
        Step::CheckedRename
    }
}

fn renameat2_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Completes the hard-link move. If the source name cannot be removed, the new
/// name is removed again (only while it still is the linked source) so the
/// move fails as a whole instead of leaving two names.
pub(super) fn remove_linked_source(source: &Path, destination: &Path) -> io::Result<()> {
    let error = match std::fs::remove_file(source) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let undo = match (
        std::fs::symlink_metadata(source),
        std::fs::symlink_metadata(destination),
    ) {
        (Ok(linked), Ok(created))
            if linked.dev() == created.dev() && linked.ino() == created.ino() =>
        {
            std::fs::remove_file(destination)
        }
        _ => Err(io::Error::other(
            "destination no longer is the linked source",
        )),
    };
    match undo {
        Ok(()) => Err(error),
        Err(undo) => Err(io::Error::new(
            error.kind(),
            format!(
                "{error}; the new name {} was kept: {undo}",
                destination.display()
            ),
        )),
    }
}

/// Last resort: refuse an existing destination, then rename.
pub(super) fn checked_rename(source: &Path, destination: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(destination) {
        Ok(_) => Err(io::Error::from_raw_os_error(libc::EEXIST)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            std::fs::rename(source, destination)
        }
        Err(error) => Err(error),
    }
}

fn record_fallback(attempt: Attempt, errno: Option<i32>) {
    let code = errno.unwrap_or(0);
    let first = {
        let mut seen = OBSERVED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if seen.contains(&(attempt, code)) {
            false
        } else {
            seen.push((attempt, code));
            true
        }
    };
    if first && crate::support_dirs::host().is_some() {
        append_log(attempt, code);
    }
}

fn append_log(attempt: Attempt, code: i32) {
    let line = format!(
        "{} {} errno {code}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        attempt.label()
    );
    let path = crate::support_dirs::app_data_file(LOG_FILE);
    // Diagnostics only: a failed log write must not fail the rename it records.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}
