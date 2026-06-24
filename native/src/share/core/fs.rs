use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::creds::{Protocol, SavedConnection};
use crate::vfs::{BackendHandle, LocalBackend, VfsMeta};
use serde::{Deserialize, Serialize};

use super::core::eio;
use super::wire::FsMeta;

const CONNECTIONS_MOUNT: &str = "Verbindungen";
pub(crate) const CHUNK: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharedRoot {
    pub label: String,
    pub path: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShareExportConfig {
    pub roots: Vec<SharedRoot>,
    pub include_connections: bool,
}

#[derive(Clone)]
enum MountTarget {
    Local(String),
    Connection(SavedConnection),
}

#[derive(Clone)]
struct Mount {
    name: String,
    target: MountTarget,
}

pub(crate) struct ResolvedTarget {
    pub(crate) backend: BackendHandle,
    pub(crate) path: String,
    pub(crate) mount_key: String,
    _net: Option<crate::net::NetConnection>,
}

pub(crate) fn list_dir(
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<Vec<FsMeta>> {
    let parts = split_clean(path)?;
    if parts.is_empty() {
        let cfg = snapshot(exports);
        let mut out: Vec<FsMeta> = local_mounts(&cfg)
            .into_iter()
            .map(|m| dir_meta(m.name))
            .collect();
        if cfg.include_connections && !connection_mounts().is_empty() {
            out.push(dir_meta(CONNECTIONS_MOUNT.to_string()));
        }
        return Ok(out);
    }
    if parts.len() == 1 && parts[0] == CONNECTIONS_MOUNT {
        if !snapshot(exports).include_connections {
            return Err(eio("Eigene Verbindungen sind nicht freigegeben"));
        }
        return Ok(connection_mounts()
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
    let parts = split_clean(path)?;
    if parts.is_empty() {
        return Ok(dir_meta("/".to_string()));
    }
    if parts.len() == 1 {
        if parts[0] == CONNECTIONS_MOUNT && snapshot(exports).include_connections {
            return Ok(dir_meta(CONNECTIONS_MOUNT.to_string()));
        }
        if local_mounts(&snapshot(exports))
            .into_iter()
            .any(|m| m.name == parts[0])
        {
            return Ok(dir_meta(parts[0].clone()));
        }
    }
    if parts.len() == 2
        && parts[0] == CONNECTIONS_MOUNT
        && snapshot(exports).include_connections
        && connection_mounts().into_iter().any(|m| m.name == parts[1])
    {
        return Ok(dir_meta(parts[1].clone()));
    }
    let t = resolve(path, exports)?;
    Ok(t.backend.stat(&t.path)?.into())
}

pub(crate) fn resolve(
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<ResolvedTarget> {
    let parts = split_clean(path)?;
    let (head, rest) = parts
        .split_first()
        .ok_or_else(|| eio("Wurzel ist kein Datei-Ziel"))?;
    if head == CONNECTIONS_MOUNT {
        if !snapshot(exports).include_connections {
            return Err(eio("Eigene Verbindungen sind nicht freigegeben"));
        }
        let (conn_name, conn_rest) = rest.split_first().ok_or_else(|| eio("Verbindung fehlt"))?;
        let mount = connection_mounts()
            .into_iter()
            .find(|m| m.name == *conn_name)
            .ok_or_else(|| eio("Unbekannte Verbindung"))?;
        let MountTarget::Connection(c) = mount.target else {
            return Err(eio("Ungueltiges Verbindungsziel"));
        };
        return resolve_connection(&c, conn_rest);
    }

    let mount = local_mounts(&snapshot(exports))
        .into_iter()
        .find(|m| m.name == *head)
        .ok_or_else(|| eio("Unbekannte Freigabe"))?;
    let MountTarget::Local(root) = mount.target else {
        return Err(eio("Ungueltiges Freigabeziel"));
    };
    let target = secure_local_target(&root, rest)?;
    Ok(ResolvedTarget {
        backend: Arc::new(LocalBackend::new(&root)),
        path: target,
        mount_key: format!("local:{head}"),
        _net: None,
    })
}

fn resolve_connection(c: &SavedConnection, rest: &[String]) -> io::Result<ResolvedTarget> {
    if c.protocol == Protocol::Share {
        let secret = crate::creds::get_secret(&c.account());
        let nc = crate::net::NetConnection::connect(
            &c.root,
            opt(&c.user).as_deref(),
            secret.as_deref(),
        )?;
        let root = c.root.replace('\\', "/");
        let path = secure_local_target(&root, rest)?;
        return Ok(ResolvedTarget {
            backend: Arc::new(LocalBackend::new(&root)),
            path,
            mount_key: c.account(),
            _net: Some(nc),
        });
    }

    let target = join_under(&norm_root(&c.root), rest);
    let (backend, root) = crate::connect::open_saved_at(c, &target).map_err(eio)?;
    Ok(ResolvedTarget {
        backend,
        path: root,
        mount_key: c.account(),
        _net: None,
    })
}

pub(crate) fn remove_dir_recursive(be: &dyn crate::vfs::Backend, path: &str) -> io::Result<()> {
    for entry in be.list_dir(path)? {
        let child = format!("{}/{}", path.trim_end_matches('/'), entry.name);
        if entry.is_dir {
            remove_dir_recursive(be, &child)?;
        } else {
            be.remove_file_id(&child, entry.id.as_deref())?;
        }
    }
    be.remove_dir(path)
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
    let mut used = Vec::new();
    if cfg.include_connections {
        used.push(CONNECTIONS_MOUNT.to_string());
    }
    cfg.roots
        .iter()
        .filter_map(|r| {
            let path = r.path.trim();
            if path.is_empty() {
                return None;
            }
            Some(Mount {
                name: unique_name(&mut used, &r.label),
                target: MountTarget::Local(path.replace('\\', "/")),
            })
        })
        .collect()
}

fn connection_mounts() -> Vec<Mount> {
    let mut used = Vec::new();
    crate::creds::load_connections()
        .into_iter()
        .map(|c| Mount {
            name: unique_name(&mut used, &c.display()),
            target: MountTarget::Connection(c),
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

fn split_clean(path: &str) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    for p in path.trim().trim_matches('/').split('/') {
        if p.is_empty() {
            continue;
        }
        if p == "." || p == ".." || p.contains('\\') || p.contains('\0') {
            return Err(eio("Ungueltiger Pfad"));
        }
        out.push(p.to_string());
    }
    Ok(out)
}

fn join_under(root: &str, rest: &[String]) -> String {
    let root = root.replace('\\', "/");
    if rest.is_empty() {
        return norm_root(&root);
    }
    let base = norm_root(&root);
    format!("{}/{}", base.trim_end_matches('/'), rest.join("/"))
}

fn secure_local_target(root: &str, rest: &[String]) -> io::Result<String> {
    let root_norm = norm_root(root);
    let root_os = to_os_path(&root_norm);
    let root_canon = std::fs::canonicalize(&root_os)
        .map_err(|e| eio(format!("Freigabe-Wurzel kann nicht gelesen werden: {e}")))?;
    let target_os = rest
        .iter()
        .fold(root_canon.clone(), |p, segment| p.join(segment));
    ensure_under_root(&root_canon, &target_os)?;
    Ok(from_os_path(&target_os))
}

fn ensure_under_root(root_canon: &Path, target: &Path) -> io::Result<()> {
    if target.exists() {
        let target_canon = std::fs::canonicalize(target)?;
        if !target_canon.starts_with(root_canon) {
            return Err(eio("Symlink/Reparse-Point fuehrt aus der Freigabe heraus"));
        }
        return Ok(());
    }

    let mut existing = target;
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| eio("Ziel hat keinen gueltigen Elternordner"))?;
    }
    let existing_canon = std::fs::canonicalize(existing)?;
    if !existing_canon.starts_with(root_canon) {
        return Err(eio("Ziel liegt ausserhalb der Freigabe"));
    }
    Ok(())
}

fn to_os_path(path: &str) -> PathBuf {
    let b = path.as_bytes();
    let rooted;
    let path = if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        rooted = format!("{path}/");
        rooted.as_str()
    } else {
        path
    };
    if std::path::MAIN_SEPARATOR == '/' {
        PathBuf::from(path)
    } else {
        PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
    }
}

fn from_os_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn norm_root(root: &str) -> String {
    let r = root.trim().replace('\\', "/");
    if r.is_empty() {
        return "/".to_string();
    }
    let b = r.as_bytes();
    if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return format!("{r}/");
    }
    if b.len() == 3 && b[1] == b':' && b[2] == b'/' && b[0].is_ascii_alphabetic() {
        return r;
    }
    let trimmed = r.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{secure_local_target, split_clean, to_os_path};

    #[test]
    fn split_clean_blocks_traversal() {
        assert!(split_clean("/root/../secret").is_err());
        assert!(split_clean("/root\\secret").is_err());
        assert!(split_clean("/root/ok").is_ok());
    }

    #[test]
    fn local_target_stays_under_root() {
        let root = std::env::temp_dir().join(format!("se-share-root-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let root_s = root.to_string_lossy().replace('\\', "/");
        let p = secure_local_target(&root_s, &["sub".to_string(), "file.txt".to_string()]).unwrap();
        let p = to_os_path(&p);
        let parent = p.parent().unwrap().canonicalize().unwrap();
        assert!(parent.starts_with(root.canonicalize().unwrap()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn symlink_escape_is_blocked_when_supported() {
        let base = std::env::temp_dir().join(format!("se-share-symlink-{}", std::process::id()));
        let root = base.join("root");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let link = root.join("link");

        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&outside, &link);
        #[cfg(not(windows))]
        let linked = std::os::unix::fs::symlink(&outside, &link);

        if linked.is_ok() {
            let root_s = root.to_string_lossy().replace('\\', "/");
            assert!(secure_local_target(&root_s, &["link".to_string()]).is_err());
        }
        let _ = std::fs::remove_dir_all(base);
    }
}
