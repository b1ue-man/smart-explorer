//! Replacing a local file on Unix-like systems. Below vfs and support_dirs
//! (plan docs/plaene/2026-11-01-idealzustand, D13); vfs re-exports it.
use std::path::Path;

/// Atomically replace `destination` (a regular file the caller checked) with
/// `source`.
pub fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}
