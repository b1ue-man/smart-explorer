#[cfg(not(windows))]
pub(super) fn should_skip_meta(name: &str, _meta: &std::fs::Metadata) -> bool {
    super::filters::should_skip(name)
}

#[cfg(not(windows))]
pub(super) fn is_plain_directory(meta: &std::fs::Metadata) -> bool {
    meta.is_dir() && !meta.file_type().is_symlink()
}

#[cfg(not(windows))]
pub(super) fn replace_file(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}
