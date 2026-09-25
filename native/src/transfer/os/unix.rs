//! Unix (Linux, Android) filesystem facts for transfers.
use std::path::Path;

/// Uploads never follow links: a symlink source is refused, not resolved.
pub(crate) fn upload_is_link_like(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(crate) fn replace_file_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(src, dest)
}

/// No free-space probe here; the download preflight is then skipped.
pub(crate) fn available_space_for_path(_path: &Path) -> Option<u64> {
    None
}
