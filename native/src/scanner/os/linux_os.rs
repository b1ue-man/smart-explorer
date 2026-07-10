#[cfg(not(windows))]
pub(super) fn get_attrs(_meta: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}

#[cfg(not(windows))]
pub(super) fn is_link_like(meta: &std::fs::Metadata) -> bool {
    meta.is_symlink()
}

#[cfg(not(windows))]
pub(super) fn path_text(path: &std::path::Path) -> Option<String> {
    path.to_str().map(str::to_owned)
}
