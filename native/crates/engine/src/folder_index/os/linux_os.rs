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

/// Android lists other apps' folders (`Android/data/*`, `obb/*`) but denies
/// opening them even with all-files access, as "permission denied" or, on
/// FUSE storage, "no such file"; the index leaves such folders out instead of
/// failing. Linux keeps failing the build on any unreadable folder.
#[cfg(not(windows))]
pub(super) fn skip_unreadable_directory(error: &std::io::Error) -> bool {
    cfg!(target_os = "android")
        && matches!(
            error.kind(),
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
        )
}
