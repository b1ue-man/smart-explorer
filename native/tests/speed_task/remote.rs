//! The servers of the speed cases (U55-5): the six ways, connected through the
//! public entry points the app uses, and the side door that puts test data
//! onto a server without crossing the shaped link (`docker cp` into the
//! container, or the export folder of a Share device on the same machine).
//!
//! Variables (U55-3; the fingerprint, container and Share ones are additions
//! of the stage, see `native/rdt_task/speed_net.py`):
//! - `SE_SPEED_SFTP`, `SE_SPEED_AGENT`: `host:port:user:password:/root`, with
//!   `…_FINGERPRINTS` (SHA256 host keys, space separated);
//! - `SE_SPEED_FTP_URL` `ftp://u:p@host:port/root`, `SE_SPEED_WEBDAV_URL`
//!   `http://u:p@host:port/root` (plain HTTP: the fixture CA hook of the
//!   WebDAV transport exists only in library tests), `SE_SPEED_SMB_URL`
//!   `smb://u:p@host:port/share`;
//! - `SE_SPEED_<WAY>_CONTAINER` `name:/dir` names the container and the
//!   folder inside it that is the server root;
//! - `SE_SPEED_SHARE_ENDPOINT` / `SE_SPEED_SHARE_DIR`: the export of the other
//!   device and its folder on this machine.
#![allow(dead_code)]

use crate::support::{need, remote_folder, var};
use crate::tools::{run_tool, strings};
use smart_explorer::creds::{save_connection_with_secret, AuthKind, Protocol, SavedConnection};
use smart_explorer::vfs::{Backend, BackendHandle};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    Sftp,
    Ftp,
    Webdav,
    Smb,
    Share,
    Agent,
}

impl Way {
    pub fn parse(name: &str) -> Way {
        match name {
            "sftp" => Way::Sftp,
            "ftp" => Way::Ftp,
            "webdav" => Way::Webdav,
            "smb" => Way::Smb,
            "share" => Way::Share,
            "agent" => Way::Agent,
            other => panic!("unknown way {other}"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Way::Sftp => "sftp",
            Way::Ftp => "ftp",
            Way::Webdav => "webdav",
            Way::Smb => "smb",
            Way::Share => "share",
            Way::Agent => "agent",
        }
    }

    /// Whether the way runs through Docker's published port (a docker-proxy
    /// hop). The capacity of that path is `proxy_tcp_bps` (M-1).
    pub fn proxied(self) -> bool {
        matches!(self, Way::Sftp | Way::Webdav | Way::Smb | Way::Agent)
    }
}

/// Host, port and login of a server, for the saved connection and rclone.
#[derive(Clone, Debug)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub root: String,
}

/// `host:port:user:password:/root`.
pub fn parse_colon_spec(spec: &str) -> Result<Server, String> {
    let parts: Vec<&str> = spec.splitn(5, ':').collect();
    if parts.len() != 5 {
        return Err("die Angabe ist nicht host:port:user:password:/root".into());
    }
    Ok(Server {
        host: parts[0].into(),
        port: parts[1]
            .parse()
            .map_err(|_| "ungültiger Port".to_string())?,
        user: parts[2].into(),
        password: parts[3].into(),
        root: parts[4].into(),
    })
}

/// `scheme://user:password@host:port/root`.
pub fn parse_url(url: &str) -> Result<Server, String> {
    let (_, rest) = url.split_once("://").ok_or("die URL hat kein Schema")?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let (userinfo, hostport) = authority
        .rsplit_once('@')
        .ok_or("die URL hat keine Anmeldung")?;
    let (user, password) = userinfo
        .split_once(':')
        .ok_or("die URL hat kein Kennwort")?;
    let (host, port) = hostport.rsplit_once(':').ok_or("die URL hat keinen Port")?;
    Ok(Server {
        host: host.into(),
        port: port.parse().map_err(|_| "ungültiger Port".to_string())?,
        user: user.into(),
        password: password.into(),
        root: path,
    })
}

/// Where test data is put without crossing the shaped link.
enum Seed {
    Container { name: String, dir: String },
    Host(PathBuf),
}

/// A connected server: backend, writable root, side door.
pub struct Remote {
    pub way: Way,
    pub backend: BackendHandle,
    pub root: String,
    pub server: Option<Server>,
    seed: Option<Seed>,
}

fn container_seed(variable: &str) -> Option<Seed> {
    let spec = var(variable)?;
    let (name, dir) = spec.split_once(':')?;
    Some(Seed::Container {
        name: name.into(),
        dir: dir.trim_end_matches('/').into(),
    })
}

/// Confirms the SSH host key of a fresh container once per process: a
/// fingerprint of another key type is refused without storing anything.
fn confirm_host_key(host: &str, port: u16, fingerprints: &str) -> Result<(), String> {
    static DONE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let done = DONE.get_or_init(|| Mutex::new(HashSet::new()));
    let mut done = done
        .lock()
        .map_err(|_| "host key list poisoned".to_string())?;
    let key = format!("{host}:{port}");
    if done.contains(&key) {
        return Ok(());
    }
    let mut refused = Vec::new();
    for fingerprint in fingerprints.split_whitespace() {
        match smart_explorer::sftp::trust_host_key(host, port, fingerprint, true) {
            Ok(_) => {
                done.insert(key);
                return Ok(());
            }
            Err(error) => refused.push(format!("{fingerprint}: {error}")),
        }
    }
    Err(format!(
        "kein Host-Schlüssel von {host}:{port} bestätigt: {}",
        refused.join("; ")
    ))
}

/// Saves the connection like the app does and reopens it through the shared
/// location resolver: the path of every other consumer.
fn open_saved(
    protocol: Protocol,
    server: &Server,
    agent: bool,
) -> Result<(BackendHandle, String), String> {
    let saved = SavedConnection {
        protocol,
        host: server.host.clone(),
        port: server.port,
        user: server.user.clone(),
        auth: AuthKind::Password,
        root: server.root.clone(),
        label: format!("speed-{}-{}", protocol.as_str(), server.port),
        use_agent: agent,
    };
    save_connection_with_secret(&saved, Some(&server.password))
        .map_err(|error| format!("Verbindung speichern: {error}"))?;
    smart_explorer::connect::resolve_endpoint(&saved.to_target())
}

/// An SFTP server named by two variables (the default server, a server limited
/// to one cipher, a throttled one).
pub fn connect_sftp_vars(
    spec_var: &str,
    fingerprint_var: &str,
    container_var: &str,
) -> Result<Remote, String> {
    let server = parse_colon_spec(&need(spec_var, "SFTP-Server der Stufe")?)
        .map_err(|error| format!("{spec_var}: {error}"))?;
    confirm_host_key(
        &server.host,
        server.port,
        &need(fingerprint_var, "Host-Schlüssel des Servers")?,
    )?;
    let config = smart_explorer::sftp::SftpConfig {
        host: server.host.clone(),
        port: server.port,
        user: server.user.clone(),
        auth: smart_explorer::sftp::SftpAuth::Password(server.password.clone()),
        root: server.root.clone(),
    };
    let backend = smart_explorer::sftp::SftpBackend::connect(config).map_err(|error| {
        format!(
            "SFTP {}:{} verbindet nicht: {error:?}",
            server.host, server.port
        )
    })?;
    Ok(Remote {
        way: Way::Sftp,
        backend: Arc::new(backend),
        root: server.root.clone(),
        seed: container_seed(container_var),
        server: Some(server),
    })
}

fn connect_agent() -> Result<Remote, String> {
    let server = parse_colon_spec(&need("SE_SPEED_AGENT", "SSH-Server des Agent-Weges")?)
        .map_err(|error| format!("SE_SPEED_AGENT: {error}"))?;
    confirm_host_key(
        &server.host,
        server.port,
        &need(
            "SE_SPEED_AGENT_FINGERPRINTS",
            "Host-Schlüssel des Agent-Servers",
        )?,
    )?;
    let (backend, root) = open_saved(Protocol::Sftp, &server, true)?;
    assert!(
        backend.supports_walk_hashed() && backend.supports_walk_tree(),
        "der Agent wurde nicht bereitgestellt: die Verbindung fiel auf einfaches SFTP zurück"
    );
    Ok(Remote {
        way: Way::Agent,
        backend,
        root,
        seed: container_seed("SE_SPEED_AGENT_CONTAINER"),
        server: Some(server),
    })
}

fn connect_share(endpoint_var: &str, dir_var: &str) -> Result<Remote, String> {
    let endpoint = need(endpoint_var, "Share-Gerätepaar der Stufe")?;
    let dir = need(dir_var, "Exportordner des Wirts")?;
    let (backend, root) = smart_explorer::connect::resolve_endpoint(&endpoint)?;
    Ok(Remote {
        way: Way::Share,
        backend,
        root,
        server: None,
        seed: Some(Seed::Host(PathBuf::from(dir))),
    })
}

/// The Share room of the stage (`room.rs`).
pub fn connect_room() -> Result<Remote, String> {
    connect_share("SE_SPEED_ROOM_ENDPOINT", "SE_SPEED_ROOM_DIR")
}

pub fn connect(way: Way) -> Result<Remote, String> {
    match way {
        Way::Sftp => connect_sftp_vars(
            "SE_SPEED_SFTP",
            "SE_SPEED_SFTP_FINGERPRINTS",
            "SE_SPEED_SFTP_CONTAINER",
        ),
        Way::Agent => connect_agent(),
        Way::Share => connect_share("SE_SPEED_SHARE_ENDPOINT", "SE_SPEED_SHARE_DIR"),
        Way::Ftp => {
            let url = need("SE_SPEED_FTP_URL", "FTP-Server der Stufe")?;
            let server = parse_url(&url).map_err(|error| format!("SE_SPEED_FTP_URL: {error}"))?;
            let backend = smart_explorer::ftp::backend_from_url(&url)
                .map_err(|error| format!("FTP verbindet nicht: {error}"))?;
            Ok(Remote {
                way,
                backend: Arc::new(backend),
                root: server.root.clone(),
                seed: container_seed("SE_SPEED_FTP_CONTAINER"),
                server: Some(server),
            })
        }
        Way::Webdav => {
            let url = need("SE_SPEED_WEBDAV_URL", "WebDAV-Server der Stufe")?;
            let server =
                parse_url(&url).map_err(|error| format!("SE_SPEED_WEBDAV_URL: {error}"))?;
            let config = smart_explorer::webdav::WebdavConfig {
                https: false,
                host: server.host.clone(),
                port: server.port,
                user: server.user.clone(),
                password: server.password.clone(),
                root: server.root.clone(),
            };
            let backend = smart_explorer::webdav::WebdavBackend::connect(config)
                .map_err(|error| format!("WebDAV verbindet nicht: {error}"))?;
            Ok(Remote {
                way,
                backend: Arc::new(backend),
                root: server.root.clone(),
                seed: container_seed("SE_SPEED_WEBDAV_CONTAINER"),
                server: Some(server),
            })
        }
        Way::Smb => {
            let url = need("SE_SPEED_SMB_URL", "SMB-Freigabe der Stufe")?;
            let server = parse_url(&url).map_err(|error| format!("SE_SPEED_SMB_URL: {error}"))?;
            let (backend, root) = open_saved(Protocol::Smb, &server, false)?;
            Ok(Remote {
                way,
                backend,
                root,
                seed: container_seed("SE_SPEED_SMB_CONTAINER"),
                server: Some(server),
            })
        }
    }
}

/// A case folder on the server that is removed from the server's disk when
/// the case ends (also on a panic).
pub struct FolderGuard<'a> {
    remote: &'a Remote,
    pub path: String,
}

impl Drop for FolderGuard<'_> {
    fn drop(&mut self) {
        self.remote.cleanup(&self.path);
    }
}

const SEED_LIMIT: Duration = Duration::from_secs(900);

impl Remote {
    /// A fresh folder below the server root; returns its backend path.
    pub fn folder(&self, name: &str) -> String {
        remote_folder(&*self.backend, &self.root, name)
    }

    /// A fresh folder that is cleaned up from the server's disk on drop.
    pub fn guarded_folder(&self, name: &str) -> FolderGuard<'_> {
        FolderGuard {
            remote: self,
            path: self.folder(name),
        }
    }

    /// The backend path of `name` in `folder`.
    pub fn join(&self, folder: &str, name: &str) -> String {
        format!("{}/{name}", folder.trim_end_matches('/'))
    }

    /// `folder` relative to the server root.
    pub fn rel(&self, folder: &str) -> String {
        let root = self.root.trim_end_matches('/');
        folder
            .strip_prefix(root)
            .unwrap_or(folder)
            .trim_start_matches('/')
            .to_string()
    }

    fn seed_target(&self, folder: &str) -> Result<Seed, String> {
        let rel = self.rel(folder);
        match self.seed.as_ref().ok_or_else(|| {
            format!(
                "keine Seitentür zum Server {} (SE_SPEED_{}_CONTAINER fehlt)",
                self.way.name(),
                self.way.name().to_uppercase()
            )
        })? {
            Seed::Host(dir) => Ok(Seed::Host(dir.join(rel))),
            Seed::Container { name, dir } => Ok(Seed::Container {
                name: name.clone(),
                dir: if rel.is_empty() {
                    dir.clone()
                } else {
                    format!("{dir}/{rel}")
                },
            }),
        }
    }

    /// Puts the local file into `folder` under `name` on the server's own disk.
    pub fn seed_file(&self, folder: &str, local: &Path, name: &str) -> Result<(), String> {
        let from = local.to_string_lossy().to_string();
        match self.seed_target(folder)? {
            Seed::Host(dir) => {
                std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
                std::fs::copy(local, dir.join(name)).map_err(|error| error.to_string())?;
            }
            Seed::Container {
                name: container,
                dir,
            } => {
                let done = run_tool(
                    "docker",
                    &strings(&["cp", &from, &format!("{container}:{dir}/{name}")]),
                    &[],
                    SEED_LIMIT,
                )?;
                if !done.ok() {
                    return Err(format!("docker cp: {}", done.describe()));
                }
            }
        }
        self.backend.invalidate_cache();
        Ok(())
    }

    /// Puts the contents of the local folder into `folder` on the server.
    pub fn seed_tree(&self, folder: &str, local: &Path) -> Result<(), String> {
        let from = format!("{}/.", local.to_string_lossy());
        let done = match self.seed_target(folder)? {
            Seed::Host(dir) => {
                std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
                run_tool(
                    "cp",
                    &strings(&["-a", &from, &dir.to_string_lossy()]),
                    &[],
                    SEED_LIMIT,
                )?
            }
            Seed::Container { name, dir } => run_tool(
                "docker",
                &strings(&["cp", &from, &format!("{name}:{dir}")]),
                &[],
                SEED_LIMIT,
            )?,
        };
        if !done.ok() {
            return Err(format!("Baum auf den Server legen: {}", done.describe()));
        }
        self.backend.invalidate_cache();
        Ok(())
    }

    /// Removes a case folder from the server's disk (best effort, so the
    /// large files of many cases do not fill the runner).
    pub fn cleanup(&self, folder: &str) {
        if self.rel(folder).is_empty() {
            return;
        }
        let _ = match self.seed_target(folder) {
            Ok(Seed::Host(dir)) => std::fs::remove_dir_all(dir).map_err(|error| error.to_string()),
            Ok(Seed::Container { name, dir }) => run_tool(
                "docker",
                &strings(&["exec", &name, "rm", "-rf", &dir]),
                &[],
                Duration::from_secs(300),
            )
            .map(|_| ()),
            Err(error) => Err(error),
        };
        self.backend.invalidate_cache();
    }
}
