//! Protected walk boundaries shared with the quick mirror.
use crate::vfs::{Backend, VfsMeta};
use super::omissions::OmissionKind;
use super::paths::{is_engine_name, rel_of};

pub(crate) fn own_path(backend: &dyn Backend, path: &str) -> bool {
    if !backend.is_local() { return false; }
    let path = std::path::Path::new(path);
    if path.starts_with(crate::support_dirs::app_data_dir()) { return true; }
    if let Some(host) = crate::support_dirs::host() {
        if !host.cache_dir.as_os_str().is_empty() && path.starts_with(&host.cache_dir) { return true; }
    }
    if let Some(cache) = std::env::var_os("XDG_CACHE_HOME") {
        if path.starts_with(std::path::Path::new(&cache).join("smart_explorer")) { return true; }
    }
    std::env::var_os("HOME").is_some_and(|home|
        path.starts_with(std::path::Path::new(&home).join(".cache/smart_explorer")))
}

pub(crate) fn protected(
    backend: &dyn Backend, root: &str, path: &str, meta: &VfsMeta,
    cross_mounts: bool,
) -> std::io::Result<Option<OmissionKind>> {
    if is_engine_name(&meta.name) || crate::vfs::is_staging_name(&meta.name)
        || own_path(backend, path) || crate::apptrash::excluded_name(&meta.name)
        || crate::apptrash::hidden_app_folders_in(path.rsplit_once('/').map_or("", |p| p.0))
    { return Ok(Some(OmissionKind::OwnFile)); }
    if meta.is_symlink { return Ok(Some(OmissionKind::Link)); }
    if meta.special { return Ok(Some(OmissionKind::Special)); }
    let rel = rel_of(path, root);
    if meta.is_dir && !rel.contains('/') && volume_root(backend, root)
        && matches!(meta.name.as_str(), "lost+found" | "System Volume Information" | "$RECYCLE.BIN")
    { return Ok(Some(OmissionKind::SystemFolder)); }
    if meta.is_dir && backend.is_local() {
        let mounted = crate::vfs::local_mount_boundary(path)?.is_some();
        if super::snapshot_mounts::missing(backend, root, path, mounted)? || (mounted && !cross_mounts) {
            return Ok(Some(OmissionKind::Mount));
        }
    }
    Ok(None)
}

fn volume_root(backend: &dyn Backend, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    root.is_empty() || (root.len() == 2 && root.as_bytes()[1] == b':')
        || (backend.is_local() && crate::vfs::local_mount_boundary(root).ok().flatten().is_some())
}
