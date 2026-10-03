//! Unix (Linux, Android) filesystem facts for transfers.
use std::path::{Path, PathBuf};

/// Uploads never follow links: a symlink source is refused, not resolved.
pub(crate) fn upload_is_link_like(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// `\` is an ordinary character in Unix file names.
pub(crate) fn backslash_is_name_char() -> bool {
    true
}

pub(crate) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(src, dest)
}

/// The folder `path` resolves to, links on the way followed.
pub(crate) fn canonical_folder(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// No free-space probe here; the download preflight is then skipped.
pub(crate) fn available_space_for_path(_path: &Path) -> Option<u64> {
    None
}

/// Memory currently available to programs (`MemAvailable` on Linux and
/// Android), for the transfer budget.
pub(crate) fn available_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Total physical memory (`MemTotal` on Linux and Android). Stable from run to
/// run, unlike the available memory, so limits derived from it do not change
/// with the momentary load (sync tree limits, RV1).
pub(crate) fn physical_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}
