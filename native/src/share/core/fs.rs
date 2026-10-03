use std::io;
use std::sync::{Arc, Mutex};

use crate::creds::{Protocol, SavedConnection};
use crate::vfs::{BackendHandle, LocalBackend, VfsMeta};

use super::core::eio;
use super::export_config::ExportAccess;
use super::fs_paths::norm_root;
pub(super) use super::fs_local_paths as local_paths;
pub(super) use local_paths::secure_local_target;
pub(super) use super::fs_paths::{join_under, split_clean};
use super::wire::FsMeta;

/// The export types live in `export_config.rs` (V2); their former paths stay.
pub use super::export_config::{ShareExportConfig, SharedRoot};

const CONNECTIONS_MOUNT: &str = "Verbindungen";
pub(crate) const CHUNK: usize = 256 * 1024;

#[derive(Clone)]
enum MountTarget {
    Local {
        path: String,
        access: ExportAccess,
        allow_system_writes: bool,
    },
    Connection {
        connection: SavedConnection,
        access: ExportAccess,
    },
}

#[derive(Clone)]
struct Mount {
    name: String,
    target: MountTarget,
}

#[derive(Clone)]
pub(crate) struct ResolvedTarget {
    pub(crate) backend: BackendHandle,
    pub(crate) path: String,
    pub(crate) mount_key: String,
    /// Access peers have to the export (or exported connection) that holds
    /// `path`; writes need `ReadWrite` (FC1, enforced by the dispatcher).
    pub(crate) access: ExportAccess,
    pub(super) allow_system_writes: bool,
    _net: Option<crate::net::NetConnection>,
}

pub(crate) fn list_dir(
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<Vec<FsMeta>> {
    if super::fs_policy::private_path(path) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Pfad ist nicht freigegeben"));
    }
    let parts = split_clean(path)?;
    if parts.is_empty() {
        let cfg = snapshot(exports);
        let mut out: Vec<FsMeta> = local_mounts(&cfg)
            .into_iter()
            .map(|m| dir_meta(m.name))
            .collect();
        if !connection_mounts(&cfg).is_empty() {
            out.push(dir_meta(CONNECTIONS_MOUNT.to_string()));
        }
        return Ok(out);
    }
    if parts.len() == 1 && parts[0] == CONNECTIONS_MOUNT {
        let cfg = snapshot(exports);
        if !cfg.shares_connections() {
            return Err(eio("Eigene Verbindungen sind nicht freigegeben"));
        }
        return Ok(connection_mounts(&cfg)
            .into_iter()
            .map(|m| dir_meta(m.name))
            .collect());
    }
    let t = resolve(path, exports)?;
    Ok(t.backend
        .list_dir(&t.path)?
        .into_iter()
        .map(Into::into)
        .collect())
}

pub(crate) fn stat(path: &str, exports: &Arc<Mutex<ShareExportConfig>>) -> io::Result<FsMeta> {
    if super::fs_policy::private_path(path) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Pfad ist nicht freigegeben"));
    }
    let parts = split_clean(path)?;
    if parts.is_empty() {
        return Ok(dir_meta("/".to_string()));
    }
    if parts.len() == 1 {
        let cfg = snapshot(exports);
        if parts[0] == CONNECTIONS_MOUNT && cfg.shares_connections() {
            return Ok(dir_meta(CONNECTIONS_MOUNT.to_string()));
        }
        if local_mounts(&cfg).into_iter().any(|m| m.name == parts[0]) {
            return Ok(dir_meta(parts[0].clone()));
        }
    }
    if parts.len() == 2
        && parts[0] == CONNECTIONS_MOUNT
        && connection_mounts(&snapshot(exports))
            .into_iter()
            .any(|m| m.name == parts[1])
    {
        return Ok(dir_meta(parts[1].clone()));
    }
    let t = resolve(path, exports)?;
    Ok(t.backend.stat(&t.path)?.into())
}

pub(crate) fn rename(
    source: &str,
    destination: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
    no_replace: bool,
) -> io::Result<()> {
    let source = resolve(source, exports)?;
    let destination = resolve(destination, exports)?;
    if source.mount_key != destination.mount_key {
        return Err(eio("Quelle und Ziel liegen nicht auf derselben Freigabe"));
    }
    if no_replace {
        source
            .backend
            .rename_no_replace(&source.path, &destination.path)
    } else {
        source.backend.rename(&source.path, &destination.path)
    }
}

pub(crate) fn promote_staged(
    staged: &str,
    destination: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    let staged = resolve(staged, exports)?;
    let destination = resolve(destination, exports)?;
    if staged.mount_key != destination.mount_key {
        return Err(eio(
            "Staging-Datei und Ziel liegen nicht auf derselben Freigabe",
        ));
    }
    staged
        .backend
        .promote_staged(&staged.path, &destination.path)
}

pub(crate) fn resolve(
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<ResolvedTarget> {
    if super::fs_policy::private_path(path) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Pfad ist nicht freigegeben"));
    }
    let parts = split_clean(path)?;
    let (head, rest) = parts
        .split_first()
        .ok_or_else(|| eio("Wurzel ist kein Datei-Ziel"))?;
    let cfg = snapshot(exports);
    if head == CONNECTIONS_MOUNT {
        if !cfg.shares_connections() {
            return Err(eio("Eigene Verbindungen sind nicht freigegeben"));
        }
        let (conn_name, conn_rest) = rest.split_first().ok_or_else(|| eio("Verbindung fehlt"))?;
        let mount = connection_mounts(&cfg)
            .into_iter()
            .find(|m| m.name == *conn_name)
            .ok_or_else(|| eio("Unbekannte Verbindung"))?;
        let MountTarget::Connection { connection, access } = mount.target else {
            return Err(eio("Ungueltiges Verbindungsziel"));
        };
        return resolve_connection(&connection, conn_rest, access);
    }

    let mount = local_mounts(&cfg)
        .into_iter()
        .find(|m| m.name == *head)
        .ok_or_else(|| eio("Unbekannte Freigabe"))?;
    let MountTarget::Local { path: root, access, allow_system_writes } = mount.target else {
        return Err(eio("Ungueltiges Freigabeziel"));
    };
    let root = secure_local_target(&root, &[])?;
    let target = secure_local_target(&root, rest)?;
    guard_target(ResolvedTarget {
        backend: Arc::new(LocalBackend::new(&root)),
        path: target,
        mount_key: format!("local:{head}"),
        access,
        allow_system_writes,
        _net: None,
    }, None)
}

fn resolve_connection(
    c: &SavedConnection,
    rest: &[String],
    access: ExportAccess,
) -> io::Result<ResolvedTarget> {
    if c.protocol == Protocol::Share {
        let secret = crate::creds::get_secret_checked(&c.account()).map_err(eio)?;
        let nc = crate::net::NetConnection::connect(
            &c.root,
            opt(&c.user).as_deref(),
            secret.as_deref(),
        )?;
        let configured = c.root.replace('\\', "/");
        let root = secure_local_target(&configured, &[])?;
        let path = secure_local_target(&root, rest)?;
        return guard_target(ResolvedTarget {
            backend: Arc::new(LocalBackend::new(&root)),
            path,
            mount_key: c.account(),
            access,
            allow_system_writes: false,
            _net: Some(nc),
        }, None);
    }

    let target = join_under(&norm_root(&c.root), rest);
    let (backend, root) = crate::connect::open_saved_at(c, &target).map_err(eio)?;
    guard_target(ResolvedTarget {
        backend,
        path: root,
        mount_key: c.account(),
        access,
        allow_system_writes: false,
        _net: None,
    }, None)
}

pub(crate) fn remove_dir_recursive(be: &dyn crate::vfs::Backend, path: &str) -> io::Result<()> {
    super::fs_delete::remove_tree(be, path)
}

pub(super) fn guard_target(mut target: ResolvedTarget,
    authority: Option<Arc<super::fs_access::AccessAuthority>>) -> io::Result<ResolvedTarget> {
    let policy = super::fs_host_policy::TargetPolicy::new(target.access,
        target.allow_system_writes, target.backend.is_local()).with_root(&target.backend.root_display());
    policy.read(&target.path)?;
    target.backend = Arc::new(super::fs_guard_backend::GuardedBackend::new(target.backend, policy, authority));
    Ok(target)
}

pub(super) fn require_target_write(target: &ResolvedTarget) -> io::Result<()> {
    super::fs_host_policy::TargetPolicy::new(target.access, target.allow_system_writes,
        target.backend.is_local()).with_root(&target.backend.root_display()).write(&target.path)
}

pub(super) fn require_target_destructive(target: &ResolvedTarget) -> io::Result<()> {
    super::fs_host_policy::TargetPolicy::new(target.access, target.allow_system_writes,
        target.backend.is_local()).with_root(&target.backend.root_display()).destructive(&target.path)
}

/// Foreign analysis traversals call this immediately after opening a root or
/// child. The decision uses the held physical object, including UNC/bind aliases.
pub(in crate::share) fn ensure_local_share_handle_allowed(handle: &crate::local_access::DirectoryHandle) -> io::Result<()> {
    super::fs_host_policy::ensure_handle_allowed(handle)
}

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn snapshot(exports: &Arc<Mutex<ShareExportConfig>>) -> ShareExportConfig {
    exports.lock().map(|g| g.clone()).unwrap_or_default()
}

fn local_mounts(cfg: &ShareExportConfig) -> Vec<Mount> {
    // Reserve the protocol container unconditionally. Otherwise a local root
    // named `Verbindungen` is advertised when the container is disabled, but
    // `resolve` still interprets that spelling as the synthetic container.
    let mut used = vec![CONNECTIONS_MOUNT.to_string()];
    cfg.roots
        .iter()
        .filter_map(|r| {
            let path = r.path.trim();
            if path.is_empty() {
                return None;
            }
            Some(Mount {
                name: unique_name(&mut used, &r.label),
                target: MountTarget::Local {
                    path: path.replace('\\', "/"),
                    access: r.access,
                    allow_system_writes: r.allow_system_writes,
                },
            })
        })
        .collect()
}

/// The exported saved connections; none without loading credentials when
/// the configuration exports no connection.
fn connection_mounts(cfg: &ShareExportConfig) -> Vec<Mount> {
    if !cfg.shares_connections() {
        return Vec::new();
    }
    let mut used = Vec::new();
    crate::creds::load_connections()
        .into_iter()
        .filter_map(|connection| {
            let access = cfg.connection_access(&connection.account())?;
            Some(Mount {
                name: unique_name(&mut used, &connection.display()),
                target: MountTarget::Connection { connection, access },
            })
        })
        .collect()
}

fn unique_name(used: &mut Vec<String>, label: &str) -> String {
    let base = clean_mount_label(label);
    let mut name = base.clone();
    let mut n = 2usize;
    while used.iter().any(|u| u == &name) {
        name = format!("{base} ({n})");
        n += 1;
    }
    used.push(name.clone());
    name
}

fn clean_mount_label(label: &str) -> String {
    let mut out: String = label
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    out = out.trim_matches([' ', '.']).to_string();
    if out.is_empty() {
        "Freigabe".to_string()
    } else {
        out
    }
}

fn dir_meta(name: String) -> FsMeta {
    FsMeta {
        name,
        is_dir: true,
        is_symlink: false,
        size: 0,
        mtime_ms: 0,
        btime_ms: 0,
        hidden: false,
        system: false,
        id: None,
        special: false,
    }
}

impl From<VfsMeta> for FsMeta {
    fn from(m: VfsMeta) -> Self {
        FsMeta {
            name: m.name,
            is_dir: m.is_dir,
            is_symlink: m.is_symlink,
            size: m.size,
            mtime_ms: m.mtime_ms,
            btime_ms: m.btime_ms,
            hidden: m.hidden,
            system: m.system,
            id: m.id,
            special: m.special,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::split_clean;

    #[test]
    fn split_clean_blocks_traversal() {
        assert!(split_clean("/root/../secret").is_err());
        assert!(split_clean("/root\\secret").is_err());
        assert!(split_clean("/root/ok").is_ok());
    }
}
