use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

use crate::creds::{Protocol, SavedConnection};
use crate::vfs::{BackendHandle, LocalBackend, VfsMeta};

use super::core::eio;
use super::protocol::{Channel, TAG_CTRL, TAG_DATA};
use super::wire::{Ctrl, FsMeta, FsRequest, FsResponse};

const CONNECTIONS_MOUNT: &str = "Verbindungen";
const CHUNK: usize = 60_000;

#[derive(Clone, Debug)]
pub struct SharedRoot {
    pub label: String,
    pub path: String,
}

#[derive(Clone, Debug, Default)]
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

struct ResolvedTarget {
    backend: BackendHandle,
    path: String,
    mount_key: String,
    _net: Option<crate::net::NetConnection>,
}

pub(crate) fn handle_fs_request(
    mut ch: Channel,
    req: FsRequest,
    exports: Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    match req {
        FsRequest::ListDir { path } => match list_dir(&path, &exports) {
            Ok(entries) => reply(&mut ch, FsResponse::Entries { entries }),
            Err(e) => reply_err(&mut ch, e),
        },
        FsRequest::Stat { path } => match stat(&path, &exports) {
            Ok(meta) => reply(&mut ch, FsResponse::Meta { meta }),
            Err(e) => reply_err(&mut ch, e),
        },
        FsRequest::Read { path } => read_file(&mut ch, &path, &exports),
        FsRequest::Write { path } => write_file(&mut ch, &path, &exports),
        FsRequest::MkdirAll { path } => {
            simple(&mut ch, &path, &exports, |t| t.backend.mkdir_all(&t.path))
        }
        FsRequest::Rename { src, dst } => {
            match (resolve(&src, &exports), resolve(&dst, &exports)) {
                (Ok(a), Ok(b)) => {
                    if a.mount_key == b.mount_key {
                        match a.backend.rename(&a.path, &b.path) {
                            Ok(()) => reply(&mut ch, FsResponse::Ok),
                            Err(e) => reply_err(&mut ch, e),
                        }
                    } else {
                        reply_err(
                            &mut ch,
                            eio("Quelle und Ziel liegen nicht auf derselben Freigabe"),
                        )
                    }
                }
                (Err(e), _) | (_, Err(e)) => reply_err(&mut ch, e),
            }
        }
        FsRequest::RemoveFile { path } => {
            simple(&mut ch, &path, &exports, |t| t.backend.remove_file(&t.path))
        }
        FsRequest::RemoveDir { path } => simple(&mut ch, &path, &exports, |t| {
            remove_dir_recursive(&*t.backend, &t.path)
        }),
        FsRequest::WriteDone => reply_err(&mut ch, eio("unerwartetes Schreib-Ende")),
    }
}

fn reply(ch: &mut Channel, resp: FsResponse) -> io::Result<()> {
    ch.send(
        TAG_CTRL,
        &serde_json::to_vec(&Ctrl::FsResp { resp }).map_err(eio)?,
    )
}

fn reply_err(ch: &mut Channel, e: io::Error) -> io::Result<()> {
    reply(&mut *ch, FsResponse::Err { msg: e.to_string() })
}

fn simple<F>(
    ch: &mut Channel,
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
    f: F,
) -> io::Result<()>
where
    F: FnOnce(ResolvedTarget) -> io::Result<()>,
{
    match resolve(path, exports).and_then(f) {
        Ok(()) => reply(ch, FsResponse::Ok),
        Err(e) => reply_err(ch, e),
    }
}

fn list_dir(path: &str, exports: &Arc<Mutex<ShareExportConfig>>) -> io::Result<Vec<FsMeta>> {
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

fn stat(path: &str, exports: &Arc<Mutex<ShareExportConfig>>) -> io::Result<FsMeta> {
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

fn read_file(
    ch: &mut Channel,
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    let t = match resolve(path, exports) {
        Ok(t) => t,
        Err(e) => return reply_err(ch, e),
    };
    let size = match t.backend.stat(&t.path) {
        Ok(m) if !m.is_dir => m.size,
        Ok(_) => return reply_err(ch, eio("Ordner kann nicht als Datei gelesen werden")),
        Err(e) => return reply_err(ch, e),
    };
    let mut r = match t.backend.open_read(&t.path) {
        Ok(r) => r,
        Err(e) => return reply_err(ch, e),
    };
    reply(ch, FsResponse::Data { size })?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ch.send(TAG_DATA, &buf[..n])?;
    }
    Ok(())
}

fn write_file(
    ch: &mut Channel,
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    let t = match resolve(path, exports) {
        Ok(t) => t,
        Err(e) => return reply_err(ch, e),
    };
    let mut w = match t.backend.open_write(&t.path) {
        Ok(w) => w,
        Err(e) => return reply_err(ch, e),
    };
    reply(ch, FsResponse::Ready)?;
    loop {
        let (tag, payload) = ch.recv()?;
        if tag == TAG_DATA {
            w.write_all(&payload)?;
            continue;
        }
        if tag != TAG_CTRL {
            return reply_err(ch, eio("unerwarteter Frame beim Schreiben"));
        }
        let ctrl: Ctrl = serde_json::from_slice(&payload).map_err(eio)?;
        match ctrl {
            Ctrl::Fs {
                req: FsRequest::WriteDone,
            } => {
                w.flush()?;
                return reply(ch, FsResponse::Ok);
            }
            _ => return reply_err(ch, eio("unerwartete Steuernachricht beim Schreiben")),
        }
    }
}

fn resolve(path: &str, exports: &Arc<Mutex<ShareExportConfig>>) -> io::Result<ResolvedTarget> {
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
    let target = join_under(&root, rest);
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
        return Ok(ResolvedTarget {
            backend: Arc::new(LocalBackend::new(&root)),
            path: join_under(&root, rest),
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

fn remove_dir_recursive(be: &dyn crate::vfs::Backend, path: &str) -> io::Result<()> {
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
