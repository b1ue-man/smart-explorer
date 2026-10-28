//! Protected walk boundaries shared with the quick mirror.
use super::omissions::OmissionKind;
use super::paths::{is_engine_name, rel_of};
use crate::vfs::{Backend, VfsMeta};

pub(crate) fn own_path(backend: &dyn Backend, path: &str) -> bool {
    if !backend.is_local() {
        return false;
    }
    let path = std::path::Path::new(path);
    own_prefixes()
        .iter()
        .any(|prefix| !prefix.as_os_str().is_empty() && path.starts_with(prefix))
}

/// The environment facts the application's own folders derive from. The
/// prefixes are recomputed only when one of them changes, so the private
/// application folder is created once instead of on every checked entry.
#[derive(Clone, PartialEq, Eq)]
struct PrefixKey {
    host: bool,
    data: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    appdata: Option<std::ffi::OsString>,
    cache: Option<std::ffi::OsString>,
}

fn prefix_key() -> PrefixKey {
    PrefixKey {
        host: crate::support_dirs::host().is_some(),
        data: std::env::var_os("XDG_DATA_HOME"),
        home: std::env::var_os("HOME"),
        appdata: std::env::var_os("APPDATA"),
        cache: std::env::var_os("XDG_CACHE_HOME"),
    }
}

type Prefixes = std::sync::Arc<Vec<std::path::PathBuf>>;

fn own_prefixes() -> Prefixes {
    static CACHE: std::sync::Mutex<Option<(PrefixKey, Prefixes)>> = std::sync::Mutex::new(None);
    let key = prefix_key();
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((cached, prefixes)) = cache.as_ref() {
        if *cached == key {
            return prefixes.clone();
        }
    }
    let mut prefixes = vec![crate::support_dirs::app_data_dir()];
    if let Some(host) = crate::support_dirs::host() {
        prefixes.push(host.cache_dir.clone());
    }
    if let Some(cache_home) = &key.cache {
        prefixes.push(std::path::Path::new(cache_home).join("smart_explorer"));
    }
    if let Some(home) = &key.home {
        prefixes.push(std::path::Path::new(home).join(".cache/smart_explorer"));
    }
    let prefixes = std::sync::Arc::new(prefixes);
    *cache = Some((key, prefixes.clone()));
    prefixes
}

pub(crate) fn protected(
    backend: &dyn Backend,
    root: &str,
    path: &str,
    meta: &VfsMeta,
    cross_mounts: bool,
) -> std::io::Result<Option<OmissionKind>> {
    if is_engine_name(&meta.name)
        || crate::vfs::is_staging_name(&meta.name)
        || own_path(backend, path)
        || crate::apptrash::excluded_name(&meta.name)
        || crate::apptrash::hidden_app_folders_in(path.rsplit_once('/').map_or("", |p| p.0))
    {
        return Ok(Some(OmissionKind::OwnFile));
    }
    if meta.is_symlink {
        return Ok(Some(OmissionKind::Link));
    }
    if meta.special {
        return Ok(Some(OmissionKind::Special));
    }
    let rel = rel_of(path, root);
    if meta.is_dir
        && !rel.contains('/')
        && volume_root(backend, root)
        && matches!(
            meta.name.as_str(),
            "lost+found" | "System Volume Information" | "$RECYCLE.BIN"
        )
    {
        return Ok(Some(OmissionKind::SystemFolder));
    }
    if meta.is_dir && backend.is_local() {
        return super::apply_path_context::mount_omission(
            backend,
            root,
            path,
            cross_mounts,
            || {
                let mounted = crate::vfs::local_mount_boundary(path)?.is_some();
                let missing = super::snapshot_mounts::missing(backend, root, path, mounted)?;
                Ok((missing || (mounted && !cross_mounts)).then_some(OmissionKind::Mount))
            },
        );
    }
    Ok(None)
}

fn volume_root(backend: &dyn Backend, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    root.is_empty()
        || (root.len() == 2 && root.as_bytes()[1] == b':')
        || (backend.is_local()
            && crate::vfs::local_mount_boundary(root)
                .ok()
                .flatten()
                .is_some())
}
