//! The guard of a replacement on a remote target (`publish.rs`): the file rule
//! replaces an older target only while it still is the file the decision saw.
use crate::transfer::update_rule::{Replacing, Stamp};
use crate::vfs::Backend;
use std::io;

/// Whether the target still is the file the rule decided to replace: one look
/// at it right before the replacement, compared with the stamp of the
/// decision (size, time within the window). A target that was saved again, is
/// a folder or link now, or cannot be looked at, is not replaced (`InvalidData`,
/// `NotFound` and `Unsupported` all mean "keep both"; nothing was published).
pub(super) fn still_decided(
    target: &dyn Backend,
    destination: &str,
    decided: Option<&Replacing>,
) -> io::Result<()> {
    let Some(decided) = decided else {
        return Ok(());
    };
    let now = match target.stat(destination) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(error),
        Err(error) => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Ziel nicht prüfbar: {error}"),
            ))
        }
    };
    let regular = !now.is_dir && !now.is_symlink && !now.special;
    let stamp = Stamp {
        size: now.size,
        mtime_ms: now.mtime_ms,
    };
    if regular && decided.holds(stamp) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Ziel hat sich seit der Entscheidung geändert",
    ))
}

/// A replacement the target cannot do or that found the old file changed or
/// gone: nothing was published, the stage is published beside the old file.
pub(super) fn replacement_refused(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::InvalidData | io::ErrorKind::NotFound
    )
}
