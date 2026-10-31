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
    protected_in(backend, root, path, meta, cross_mounts, None)
}

/// The facts of one local walk that do not change from folder to folder:
/// the application's own folders, whether the root is a volume root, the
/// mount boundaries below it and its remembered nested mounts. A snapshot
/// walk builds them once (`snapshot.rs`) instead of reading the environment
/// per entry and the mount table, the canonical parent and the mount memory
/// per folder; the answers are those of `protected` without them.
pub(crate) struct LocalFacts {
    own: Prefixes,
    volume_root: std::sync::OnceLock<bool>,
    mounts: crate::vfs::LocalMountBoundaries,
    nested: super::snapshot_mounts::NestedMounts,
}

impl LocalFacts {
    pub(crate) fn new(backend: &dyn Backend, root: &str) -> Self {
        Self {
            own: own_prefixes(),
            volume_root: std::sync::OnceLock::new(),
            mounts: crate::vfs::LocalMountBoundaries::new(root),
            nested: super::snapshot_mounts::NestedMounts::new(backend, root),
        }
    }

    fn own_path(&self, path: &str) -> bool {
        let path = std::path::Path::new(path);
        self.own
            .iter()
            .any(|prefix| !prefix.as_os_str().is_empty() && path.starts_with(prefix))
    }
}

/// Whether a folder holds other apps' private storage
/// (`apptrash::hidden_app_folders_in`), asked once per folder: the entries of
/// one listing share their parent, so only a new parent is asked again.
#[derive(Default)]
pub(crate) struct AppParents {
    last: Option<(String, bool)>,
}

impl AppParents {
    fn hidden_app_folder(&mut self, parent: &str) -> bool {
        match &self.last {
            Some((known, hidden)) if known == parent => *hidden,
            _ => {
                let hidden = crate::apptrash::hidden_app_folders_in(parent);
                self.last = Some((parent.to_string(), hidden));
                hidden
            }
        }
    }
}

/// `protected`, with the facts of the walk where it has them.
pub(crate) fn protected_in(
    backend: &dyn Backend,
    root: &str,
    path: &str,
    meta: &VfsMeta,
    cross_mounts: bool,
    facts: Option<&LocalFacts>,
) -> std::io::Result<Option<OmissionKind>> {
    let mut parents = AppParents::default();
    protected_listed(backend, root, path, meta, cross_mounts, facts, &mut parents)
}

/// `protected_in` for the entries of one listing: `parents` keeps the answer
/// for their common parent folder.
pub(crate) fn protected_listed(
    backend: &dyn Backend,
    root: &str,
    path: &str,
    meta: &VfsMeta,
    cross_mounts: bool,
    facts: Option<&LocalFacts>,
    parents: &mut AppParents,
) -> std::io::Result<Option<OmissionKind>> {
    let local = backend.is_local();
    let own = match facts {
        Some(facts) if local => facts.own_path(path),
        _ => own_path(backend, path),
    };
    if is_engine_name(&meta.name)
        || crate::vfs::is_staging_name(&meta.name)
        || own
        || crate::apptrash::excluded_name(&meta.name)
        || parents.hidden_app_folder(path.rsplit_once('/').map_or("", |p| p.0))
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
        && matches!(
            meta.name.as_str(),
            "lost+found" | "System Volume Information" | "$RECYCLE.BIN"
        )
        && match facts {
            Some(facts) => *facts.volume_root.get_or_init(|| volume_root(backend, root)),
            None => volume_root(backend, root),
        }
    {
        return Ok(Some(OmissionKind::SystemFolder));
    }
    if meta.is_dir && local {
        return super::apply_path_context::mount_omission(
            backend,
            root,
            path,
            cross_mounts,
            || {
                let (mounted, missing) = match facts {
                    Some(facts) => {
                        let mounted = facts.mounts.boundary(path)?.is_some();
                        (mounted, facts.nested.missing(backend, path, mounted)?)
                    }
                    None => {
                        let mounted = crate::vfs::local_mount_boundary(path)?.is_some();
                        let missing =
                            super::snapshot_mounts::missing(backend, root, path, mounted)?;
                        (mounted, missing)
                    }
                };
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
