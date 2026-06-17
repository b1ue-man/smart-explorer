//! SFTP backend (`russh` + `russh-sftp`) implementing `vfs::Backend`.
//!
//! Auth: username/password OR keyfile (+ optional passphrase). Host keys use
//! trust-on-first-use against `%APPDATA%\smart_explorer\known_hosts_sftp.txt`
//! (accept + persist on first sight, reject on later mismatch).
//!
//! Async↔sync bridge: a private multi-threaded tokio runtime owned by the
//! backend. A worker thread continuously drives russh's background connection
//! task, while each blocking `Backend` method runs `rt.block_on(...)`. File I/O
//! is adapted to `std::io::{Read,Write}` by `block_on`-ing the tokio async reads
//! in chunks (no `SyncIoBridge` — it conflicts with this model). This keeps
//! scanner / copy / UI fully synchronous; see docs/REMOTE_LAYER_PLAN.md §1,§3.

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use russh::client;
use russh_sftp::client::SftpSession;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Runtime;

fn io_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

// ── configuration ───────────────────────────────────────────────────────────

pub enum SftpAuth {
    Password(String),
    /// Private key file path + optional passphrase. Constructed by the Connect
    /// dialog (credential store) in the connect-UI step.
    #[allow(dead_code)]
    Key {
        path: String,
        passphrase: Option<String>,
    },
}

pub struct SftpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SftpAuth,
    /// Remote start directory (forward-slash, e.g. `/home/user`).
    pub root: String,
}

/// Parsed `sftp://[user[:password]@]host[:port][/path]`.
struct SftpUrl {
    user: String,
    password: Option<String>,
    host: String,
    port: u16,
    root: String,
}

fn parse_sftp_url(url: &str) -> io::Result<SftpUrl> {
    let rest = url
        .trim()
        .strip_prefix("sftp://")
        .ok_or_else(|| io_err("kein sftp://-URL"))?;
    // authority / path
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let root = if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    };
    // [user[:password]@]host[:port]
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(ui) => match ui.find(':') {
            Some(j) => (ui[..j].to_string(), Some(ui[j + 1..].to_string())),
            None => (ui.to_string(), None),
        },
        None => return Err(io_err("SFTP-Benutzername fehlt (sftp://user@host/…)")),
    };
    if user.is_empty() {
        return Err(io_err("SFTP-Benutzername fehlt"));
    }
    let (host, port) = match hostport.rfind(':') {
        Some(k) => {
            let p = hostport[k + 1..]
                .parse::<u16>()
                .map_err(|_| io_err("ungültiger SFTP-Port"))?;
            (hostport[..k].to_string(), p)
        }
        None => (hostport.to_string(), 22),
    };
    if host.is_empty() {
        return Err(io_err("SFTP-Host fehlt"));
    }
    Ok(SftpUrl {
        user,
        password,
        host,
        port,
        root,
    })
}

/// Connect from a `sftp://` URL. A password embedded in the URL is used; without
/// one the caller must go through the Connect dialog (credential store) — wired
/// in the connect-UI step.
pub fn backend_from_url(url: &str) -> io::Result<SftpBackend> {
    let u = parse_sftp_url(url)?;
    let auth = match u.password {
        Some(p) => SftpAuth::Password(p),
        None => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "SFTP-Zugangsdaten erforderlich — bitte den Verbinden-Dialog nutzen",
            ))
        }
    };
    SftpBackend::connect(SftpConfig {
        host: u.host,
        port: u.port,
        user: u.user,
        auth,
        root: u.root,
    })
}

// ── host-key trust-on-first-use ─────────────────────────────────────────────

fn app_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("smart_explorer");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn known_hosts_path() -> PathBuf {
    app_data_dir().join("known_hosts_sftp.txt")
}

/// TOFU: accept a matching or first-seen key (persisting it), reject a changed
/// key. Returns whether russh should accept the handshake.
fn known_hosts_accept(host: &str, port: u16, key: &russh::keys::PublicKey) -> bool {
    let fp = key.fingerprint(Default::default()).to_string();
    let id = format!("{host}:{port}");
    let path = known_hosts_path();
    if let Ok(content) = std::fs::read_to_string(&path) {
        for line in content.lines() {
            let mut it = line.split_whitespace();
            if let (Some(h), Some(f)) = (it.next(), it.next()) {
                if h == id {
                    return f == fp; // known host → must match
                }
            }
        }
    }
    // unknown → accept and persist
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{id} {fp}");
    }
    true
}

// ── russh client handler ─────────────────────────────────────────────────────

struct Client {
    host: String,
    port: u16,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(known_hosts_accept(&self.host, self.port, server_public_key))
    }
}

// ── backend ──────────────────────────────────────────────────────────────────

pub struct SftpBackend {
    rt: Arc<Runtime>,
    // Kept alive so the encrypted connection (and its background task) survive.
    _session: client::Handle<Client>,
    sftp: Arc<SftpSession>,
    root: String,
    /// Read by `url()` (UI display), consumed in the connect-UI step.
    #[allow(dead_code)]
    url: String,
}

impl SftpBackend {
    pub fn connect(cfg: SftpConfig) -> io::Result<SftpBackend> {
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(io_err)?,
        );
        let url = format!("sftp://{}@{}:{}{}", cfg.user, cfg.host, cfg.port, cfg.root);
        let root = cfg.root.clone();
        let (session, sftp) = rt.block_on(connect_async(cfg))?;
        Ok(SftpBackend {
            rt,
            _session: session,
            sftp: Arc::new(sftp),
            root,
            url,
        })
    }

    /// `sftp://user@host:port/root` for UI display (connect-UI step).
    #[allow(dead_code)]
    pub fn url(&self) -> String {
        self.url.clone()
    }
}

async fn connect_async(cfg: SftpConfig) -> io::Result<(client::Handle<Client>, SftpSession)> {
    let config = Arc::new(client::Config::default());
    let handler = Client {
        host: cfg.host.clone(),
        port: cfg.port,
    };
    let mut session = client::connect(config, (cfg.host.as_str(), cfg.port), handler)
        .await
        .map_err(io_err)?;

    let authed = match &cfg.auth {
        SftpAuth::Password(pw) => session
            .authenticate_password(&cfg.user, pw)
            .await
            .map_err(io_err)?
            .success(),
        SftpAuth::Key { path, passphrase } => {
            let key = russh::keys::load_secret_key(path, passphrase.as_deref()).map_err(io_err)?;
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(io_err)?
                .flatten();
            session
                .authenticate_publickey(
                    &cfg.user,
                    russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(io_err)?
                .success()
        }
    };
    if !authed {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SFTP-Authentifizierung fehlgeschlagen",
        ));
    }

    let channel = session.channel_open_session().await.map_err(io_err)?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(io_err)?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(io_err)?;
    Ok((session, sftp))
}

fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

fn to_vfs(name: String, meta: &russh_sftp::protocol::FileAttributes) -> VfsMeta {
    let ft = meta.file_type();
    VfsMeta {
        is_dir: ft.is_dir(),
        is_symlink: ft.is_symlink(),
        size: meta.size.unwrap_or(0),
        // SFTP mtime is unix seconds; no btime / hidden / system attrs.
        mtime_ms: meta.mtime.map(|s| s as i64 * 1000).unwrap_or(0),
        btime_ms: 0,
        hidden: name.starts_with('.'),
        system: false,
        name,
        id: None,
    }
}

impl Backend for SftpBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        let dir = rt
            .block_on(async move { sftp.read_dir(p).await })
            .map_err(io_err)?;
        let mut out = Vec::new();
        for e in dir {
            let name = e.file_name();
            let meta = e.metadata();
            out.push(to_vfs(name, &meta));
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        let meta = rt
            .block_on(async move { sftp.symlink_metadata(p).await })
            .map_err(io_err)?;
        Ok(to_vfs(basename(path), &meta))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        let file = rt
            .block_on(async move { sftp.open(p).await })
            .map_err(io_err)?;
        Ok(Box::new(SftpReader {
            rt: self.rt.clone(),
            file,
        }))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        let file = rt
            .block_on(async move { sftp.create(p).await })
            .map_err(io_err)?;
        Ok(Box::new(SftpWriter {
            rt: self.rt.clone(),
            file: Some(file),
        }))
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let (s, d) = (src.to_string(), dst.to_string());
        rt.block_on(async move { sftp.rename(s, d).await })
            .map_err(io_err)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        rt.block_on(async move { sftp.remove_file(p).await })
            .map_err(io_err)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let p = path.to_string();
        rt.block_on(async move { sftp.remove_dir(p).await })
            .map_err(io_err)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let sftp = self.sftp.clone();
        let rt = self.rt.clone();
        let absolute = path.starts_with('/');
        let parts: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        rt.block_on(async move {
            let mut cur = String::new();
            for part in parts {
                if cur.is_empty() {
                    if absolute {
                        cur.push('/');
                    }
                } else {
                    cur.push('/');
                }
                cur.push_str(&part);
                // ignore "already exists"; final existence is verified below.
                let _ = sftp.create_dir(cur.clone()).await;
            }
            sftp.metadata(cur).await.map(|_| ()).map_err(io_err)
        })
    }

    fn parallelism(&self) -> usize {
        // Conservative: one SFTP session, sequential remote walk. Safe default
        // until a real-server concurrency spike (plan §"open questions").
        1
    }
}

// ── sync I/O adapters ────────────────────────────────────────────────────────

struct SftpReader {
    rt: Arc<Runtime>,
    file: russh_sftp::client::fs::File,
}

impl Read for SftpReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let rt = self.rt.clone();
        rt.block_on(async { self.file.read(buf).await })
    }
}

struct SftpWriter {
    rt: Arc<Runtime>,
    file: Option<russh_sftp::client::fs::File>,
}

impl Write for SftpWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let rt = self.rt.clone();
        let file = self.file.as_mut().ok_or_else(|| io_err("Datei geschlossen"))?;
        rt.block_on(async { file.write(buf).await })
    }
    fn flush(&mut self) -> io::Result<()> {
        let rt = self.rt.clone();
        let file = self.file.as_mut().ok_or_else(|| io_err("Datei geschlossen"))?;
        rt.block_on(async { file.flush().await })
    }
}

impl Drop for SftpWriter {
    fn drop(&mut self) {
        // Ensure the remote file is flushed/closed (std::io::copy never calls
        // flush). Best-effort.
        if let Some(mut file) = self.file.take() {
            let rt = self.rt.clone();
            let _ = rt.block_on(async { file.shutdown().await });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_full() {
        let u = parse_sftp_url("sftp://alice:secret@example.com:2222/home/alice").unwrap();
        assert_eq!(u.user, "alice");
        assert_eq!(u.password.as_deref(), Some("secret"));
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 2222);
        assert_eq!(u.root, "/home/alice");
    }

    #[test]
    fn url_defaults() {
        let u = parse_sftp_url("sftp://bob@host").unwrap();
        assert_eq!(u.user, "bob");
        assert!(u.password.is_none());
        assert_eq!(u.host, "host");
        assert_eq!(u.port, 22);
        assert_eq!(u.root, "/");

        let u2 = parse_sftp_url("sftp://bob@host/").unwrap();
        assert_eq!(u2.root, "/");
    }

    #[test]
    fn url_errors() {
        assert!(parse_sftp_url("sftp://host/path").is_err()); // no user
        assert!(parse_sftp_url("ftp://u@host").is_err()); // wrong scheme
        assert!(parse_sftp_url("sftp://u@host:notaport/").is_err());
    }

    #[test]
    fn url_without_password_needs_dialog() {
        // backend_from_url must refuse (not connect) when no password is present.
        match backend_from_url("sftp://bob@host/") {
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::PermissionDenied),
            Ok(_) => panic!("should require credentials"),
        }
    }

    #[test]
    fn basename_works() {
        assert_eq!(basename("/home/user/file.txt"), "file.txt");
        assert_eq!(basename("/home/user/"), "user");
        assert_eq!(basename("file"), "file");
    }
}
