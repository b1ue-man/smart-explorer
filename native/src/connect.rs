//! Connect orchestration: turn a `ConnectForm` (or a saved connection) into a
//! live backend, off the UI thread. Keeps app.rs thin — it only renders the
//! form and drains the result.
//!
//! Routing once connected (decided in app.rs):
//!  * SFTP / FTP / FTPS  → a `RemoteState` backend; navigation walks it via
//!    `rscan` (remote scan path).
//!  * Network share      → authenticated with `net::NetConnection`; the UNC path
//!    is then browsed by the LOCAL scanner (std::fs handles UNC), so no
//!    `RemoteState` — only the live `NetConnection` is kept alive.

use crate::creds::{AuthKind, Protocol, SavedConnection};
use crate::vfs::BackendHandle;
use crossbeam_channel::{unbounded, Receiver};
use std::sync::Arc;

/// A live remote (SFTP/FTP) session held by the app while browsing it.
pub struct RemoteState {
    pub backend: BackendHandle,
    pub label: String,
    /// `Some(version)` when an SSH remote agent is active for this session (#24);
    /// drives the "⚡ Agent" status indicator. `None` = plain backend.
    pub agent_version: Option<String>,
    /// For an opened ZIP archive: the local folder to return to when the archive
    /// is closed (⏏). `None` for real network connections.
    pub zip_return: Option<String>,
    /// The concrete SFTP backend behind this session, kept so the SSH remote
    /// agent can be activated LATER on an already-established connection
    /// (#24, runtime opt-in). `None` for non-SFTP.
    pub sftp: Option<Arc<crate::sftp::SftpBackend>>,
    /// Saved-connection account key (if this came from a saved connection), so a
    /// later agent activation can persist the choice. `None` for ad-hoc/non-SFTP.
    pub account: Option<String>,
    /// `proto://user@host:port` for this session (`None` for local/share/zip), so
    /// favourites and per-folder settings can be keyed by connection + path
    /// (a re-openable endpoint URL) rather than a bare path.
    pub endpoint_prefix: Option<String>,
}

/// Editable Connect-dialog state.
#[derive(Clone)]
pub struct ConnectForm {
    pub protocol: Protocol,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub use_key: bool,
    pub keyfile: String,
    pub passphrase: String,
    pub root: String,
    pub unc: String, // network share path
    pub save: bool,
    pub label: String,
    /// Opt-in SSH remote agent (#24); SFTP only.
    pub use_agent: bool,
}

impl Default for ConnectForm {
    fn default() -> Self {
        ConnectForm {
            protocol: Protocol::Sftp,
            host: String::new(),
            port: "22".into(),
            user: String::new(),
            password: String::new(),
            use_key: false,
            keyfile: String::new(),
            passphrase: String::new(),
            root: "/".into(),
            unc: String::new(),
            save: false,
            label: String::new(),
            use_agent: false,
        }
    }
}

impl ConnectForm {
    /// Pre-fill the form from a saved connection (the secret is loaded
    /// separately from the keyring at connect time).
    pub fn from_saved(c: &SavedConnection) -> Self {
        let (use_key, keyfile) = match &c.auth {
            AuthKind::Key { path } => (true, path.clone()),
            AuthKind::Password => (false, String::new()),
        };
        ConnectForm {
            protocol: c.protocol,
            host: c.host.clone(),
            port: c.port.to_string(),
            user: c.user.clone(),
            password: String::new(),
            use_key,
            keyfile,
            passphrase: String::new(),
            root: if c.protocol.is_url() { c.root.clone() } else { "/".into() },
            unc: if c.protocol.is_url() { String::new() } else { c.root.clone() },
            save: true,
            label: c.label.clone(),
            use_agent: c.use_agent,
        }
    }
}

/// Outcome of a connect attempt.
pub enum ConnectResult {
    Ok(Connected),
    Err(String),
}

pub struct Connected {
    /// Some for SFTP/FTP (walked via rscan); None for a share.
    pub remote: Option<RemoteState>,
    /// Some for an authenticated share (kept alive while browsing).
    pub net: Option<crate::net::NetConnection>,
    /// Navigation target: the remote root path or the UNC path.
    pub target: String,
    pub label: String,
}

/// Connect off the UI thread; the app drains the single result.
pub fn spawn_connect(form: ConnectForm, secret: Option<String>) -> Receiver<ConnectResult> {
    let (tx, rx) = unbounded();
    std::thread::Builder::new()
        .name("connect".into())
        .spawn(move || {
            let _ = tx.send(do_connect(form, secret));
        })
        .ok();
    rx
}

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn norm_root(r: &str) -> String {
    let t = r.trim();
    if t.is_empty() {
        "/".to_string()
    } else if t.starts_with('/') {
        t.to_string()
    } else {
        format!("/{t}")
    }
}

/// Percent-encode a URL userinfo component (FTP URLs).
fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// `proto://user@host:port` for URL protocols (sftp/ftp/ftps/webdav); `None` for
/// a local share. The stable per-connection prefix used to key favourites/prefs.
fn ep_prefix(form: &ConnectForm, port: u16) -> Option<String> {
    if form.protocol.is_url() {
        Some(format!(
            "{}://{}@{}:{}",
            form.protocol.as_str(),
            form.user.trim(),
            form.host.trim(),
            port
        ))
    } else {
        None
    }
}

/// Split a remote endpoint URL into its matching saved connection + the path
/// part, so a favourite/endpoint can be re-opened.
pub fn saved_and_path(url: &str) -> Option<(SavedConnection, String)> {
    let (proto, user, host, port, path) = parse_remote_url(url)?;
    let c = crate::creds::load_connections()
        .into_iter()
        .find(|c| c.protocol == proto && c.user == user && c.host == host && c.port == port)?;
    Some((c, path))
}

fn label_for(form: &ConnectForm, port: u16) -> String {
    if !form.label.trim().is_empty() {
        return form.label.trim().to_string();
    }
    match form.protocol {
        Protocol::Share => form.unc.trim().to_string(),
        _ => format!(
            "{}://{}@{}:{}",
            form.protocol.as_str(),
            form.user,
            form.host,
            port
        ),
    }
}

/// Build the `SavedConnection` metadata (no secret) for persistence.
pub fn build_saved(form: &ConnectForm, port: u16) -> SavedConnection {
    let auth = if form.use_key {
        AuthKind::Key {
            path: form.keyfile.trim().to_string(),
        }
    } else {
        AuthKind::Password
    };
    let root = match form.protocol {
        Protocol::Share => form.unc.trim().to_string(),
        _ => norm_root(&form.root),
    };
    SavedConnection {
        protocol: form.protocol,
        host: form.host.trim().to_string(),
        port,
        user: form.user.trim().to_string(),
        auth,
        root,
        label: form.label.trim().to_string(),
        use_agent: form.use_agent && form.protocol == Protocol::Sftp,
    }
}

fn persist(form: &ConnectForm, port: u16, secret: Option<&str>) {
    if !form.save {
        return;
    }
    let saved = build_saved(form, port);
    let _ = crate::creds::save_connection(&saved);
    if let Some(s) = secret {
        if !s.is_empty() {
            let _ = crate::creds::set_secret(&saved.account(), s);
        }
    }
}

fn do_connect(form: ConnectForm, secret: Option<String>) -> ConnectResult {
    let port: u16 = form
        .port
        .trim()
        .parse()
        .unwrap_or_else(|_| form.protocol.default_port());

    match form.protocol {
        Protocol::Sftp => {
            // A saved-connection secret (keyring) overrides an empty form field.
            let password = secret.clone().unwrap_or_else(|| form.password.clone());
            let passphrase = secret.clone().unwrap_or_else(|| form.passphrase.clone());
            let auth = if form.use_key {
                crate::sftp::SftpAuth::Key {
                    path: form.keyfile.trim().to_string(),
                    passphrase: opt(&passphrase),
                }
            } else {
                crate::sftp::SftpAuth::Password(password.clone())
            };
            let root = norm_root(&form.root);
            let cfg = crate::sftp::SftpConfig {
                host: form.host.trim().to_string(),
                port,
                user: form.user.trim().to_string(),
                auth,
                root: root.clone(),
            };
            match crate::sftp::SftpBackend::connect(cfg) {
                Ok(be) => {
                    let s = if form.use_key { passphrase } else { password };
                    persist(&form, port, Some(&s));
                    let label = label_for(&form, port);
                    // Opt-in: try to deploy + use the SSH remote agent (#24). Any
                    // failure (no bundled binary, no exec right, …) falls back to
                    // plain SFTP, so connecting never breaks.
                    let be_arc: Arc<crate::sftp::SftpBackend> = Arc::new(be);
                    let sftp_handle = be_arc.clone(); // kept for later agent activation
                    let account = Some(build_saved(&form, port).account());
                    let (backend, agent_version): (BackendHandle, Option<String>) = if form.use_agent
                    {
                        let inner: BackendHandle = be_arc.clone();
                        match crate::agent::deploy_over_sftp(&be_arc, inner) {
                            Ok(agent) => {
                                let ver = agent.version().to_string();
                                (Arc::new(agent), Some(ver))
                            }
                            Err(_) => (be_arc, None), // fall back to plain SFTP
                        }
                    } else {
                        (be_arc, None)
                    };
                    ConnectResult::Ok(Connected {
                        remote: Some(RemoteState {
                            backend,
                            label: label.clone(),
                            agent_version,
                            zip_return: None,
                            sftp: Some(sftp_handle),
                            account,
                            endpoint_prefix: ep_prefix(&form, port),
                        }),
                        net: None,
                        target: root,
                        label,
                    })
                }
                Err(e) => ConnectResult::Err(e.to_string()),
            }
        }
        Protocol::Ftp | Protocol::Ftps => {
            let scheme = if form.protocol == Protocol::Ftps {
                "ftps"
            } else {
                "ftp"
            };
            let password = secret.clone().unwrap_or_else(|| form.password.clone());
            let user = form.user.trim();
            let userinfo = if password.is_empty() {
                enc(user)
            } else {
                format!("{}:{}", enc(user), enc(&password))
            };
            let root = norm_root(&form.root);
            let url = format!(
                "{scheme}://{}@{}:{}{}",
                userinfo,
                form.host.trim(),
                port,
                root
            );
            match crate::ftp::backend_from_url(&url) {
                Ok(be) => {
                    persist(&form, port, Some(&password));
                    let label = label_for(&form, port);
                    ConnectResult::Ok(Connected {
                        remote: Some(RemoteState {
                            backend: Arc::new(be),
                            label: label.clone(),
                            agent_version: None,
                            zip_return: None,
                            sftp: None,
                            account: None,
                            endpoint_prefix: ep_prefix(&form, port),
                        }),
                        net: None,
                        target: root,
                        label,
                    })
                }
                Err(e) => ConnectResult::Err(e.to_string()),
            }
        }
        Protocol::Webdav => {
            let password = secret.clone().unwrap_or_else(|| form.password.clone());
            let root = norm_root(&form.root);
            let cfg = crate::webdav::WebdavConfig {
                https: true,
                host: form.host.trim().to_string(),
                port,
                user: form.user.trim().to_string(),
                password: password.clone(),
                root: root.clone(),
            };
            match crate::webdav::WebdavBackend::connect(cfg) {
                Ok(be) => {
                    persist(&form, port, Some(&password));
                    let label = label_for(&form, port);
                    ConnectResult::Ok(Connected {
                        remote: Some(RemoteState {
                            backend: Arc::new(be),
                            label: label.clone(),
                            agent_version: None,
                            zip_return: None,
                            sftp: None,
                            account: None,
                            endpoint_prefix: ep_prefix(&form, port),
                        }),
                        net: None,
                        target: root,
                        label,
                    })
                }
                Err(e) => ConnectResult::Err(e.to_string()),
            }
        }
        Protocol::Share => {
            let unc = form.unc.trim().to_string();
            let password = secret.clone().unwrap_or_else(|| form.password.clone());
            match crate::net::NetConnection::connect(&unc, opt(&form.user).as_deref(), opt(&password).as_deref())
            {
                Ok(nc) => {
                    persist(&form, port, Some(&password));
                    let label = label_for(&form, port);
                    ConnectResult::Ok(Connected {
                        remote: None,
                        net: Some(nc),
                        target: unc,
                        label,
                    })
                }
                Err(e) => ConnectResult::Err(e.to_string()),
            }
        }
    }
}

// ─── Sync endpoints (local path OR saved-connection remote URL) ──────────────

/// Is this endpoint a remote URL (`sftp://…`, `ftp://…`, `ftps://…`,
/// `webdav://…`) rather than a local/UNC path? Used by the sync runner and the
/// in-app picker to decide whether a saved connection must be re-opened.
pub fn is_remote_url(s: &str) -> bool {
    let s = s.trim();
    ["sftp://", "ftp://", "ftps://", "webdav://", "gdrive://"]
        .iter()
        .any(|p| s.starts_with(p))
}

/// Open Google Drive at `path` as a backend (uses the stored OAuth token).
/// Blocks on the network — call off the UI thread.
pub fn open_gdrive(path: &str) -> Result<(BackendHandle, String), String> {
    let be = crate::gdrive::GDriveBackend::connect(path)?;
    let root = if path.trim().is_empty() { "/".to_string() } else { path.to_string() };
    Ok((Arc::new(be), root))
}

/// Build the `gdrive://` endpoint string for a chosen Drive folder.
pub fn gdrive_endpoint(path: &str) -> String {
    let p = path.trim_start_matches('/');
    format!("gdrive:///{}", p)
}

/// Parse `proto://user@host:port/path` → its parts (path keeps its leading `/`).
fn parse_remote_url(s: &str) -> Option<(Protocol, String, String, u16, String)> {
    let s = s.trim();
    let (scheme, rest) = s.split_once("://")?;
    let proto = Protocol::parse(scheme)?;
    // rest = user@host:port/path  (path optional)
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let (user, hostport) = match authority.rsplit_once('@') {
        Some((u, hp)) => (u.to_string(), hp),
        None => (String::new(), authority),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or_else(|_| proto.default_port())),
        None => (hostport.to_string(), proto.default_port()),
    };
    Some((proto, user, host, port, if path.is_empty() { "/".into() } else { path }))
}

/// Build the saved-connection-backed endpoint URL for a chosen remote folder.
pub fn remote_endpoint(c: &SavedConnection, path: &str) -> String {
    let p = if path.is_empty() { "/" } else { path };
    format!(
        "{}://{}@{}:{}{}",
        c.protocol.as_str(),
        c.user,
        c.host,
        c.port,
        p
    )
}

/// Open a saved connection at `path` (synchronous; blocks on the network — call
/// off the UI thread). Reuses the connection's stored credentials (keyring).
/// Returns the live backend + the navigated root path.
pub fn open_saved_at(c: &SavedConnection, path: &str) -> Result<(BackendHandle, String), String> {
    if !c.protocol.is_url() {
        // Share: the UNC is browsed locally once authenticated.
        let secret = crate::creds::get_secret(&c.account());
        let mut form = ConnectForm::from_saved(c);
        form.save = false;
        match do_connect(form, secret) {
            ConnectResult::Ok(conn) => Ok((
                Arc::new(crate::vfs::LocalBackend::new(&conn.target)),
                conn.target,
            )),
            ConnectResult::Err(e) => Err(e),
        }
    } else {
        let secret = crate::creds::get_secret(&c.account());
        let mut form = ConnectForm::from_saved(c);
        form.root = if path.is_empty() { "/".into() } else { path.to_string() };
        form.save = false;
        match do_connect(form, secret) {
            ConnectResult::Ok(conn) => match conn.remote {
                Some(rs) => Ok((rs.backend, conn.target)),
                None => Err("Endpoint ist keine Remote-Verbindung".into()),
            },
            ConnectResult::Err(e) => Err(e),
        }
    }
}

/// Resolve a sync endpoint into a live backend + root. Local/UNC paths →
/// `LocalBackend`; remote URLs → re-open the matching saved connection. Blocks
/// on the network for remote endpoints, so run it off the UI thread.
pub fn resolve_endpoint(endpoint: &str) -> Result<(BackendHandle, String), String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err("Leerer Pfad".into());
    }
    if !is_remote_url(endpoint) {
        return Ok((
            Arc::new(crate::vfs::LocalBackend::new(endpoint)),
            endpoint.to_string(),
        ));
    }
    // Google Drive: gdrive:///<path> → re-open from the stored OAuth token.
    if let Some(rest) = endpoint.strip_prefix("gdrive://") {
        let path = format!("/{}", rest.trim_start_matches('/'));
        return open_gdrive(&path);
    }
    let (proto, user, host, port, path) =
        parse_remote_url(endpoint).ok_or_else(|| "Ungültige Remote-Adresse".to_string())?;
    let conns = crate::creds::load_connections();
    let c = conns
        .iter()
        .find(|c| c.protocol == proto && c.user == user && c.host == host && c.port == port)
        .ok_or_else(|| {
            "Keine gespeicherte Verbindung für diese Remote-Adresse gefunden — bitte zuerst verbinden"
                .to_string()
        })?;
    open_saved_at(c, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_url_detection_and_parse() {
        assert!(is_remote_url("sftp://u@h:22/x"));
        assert!(is_remote_url("webdav://u@h:443/dav"));
        assert!(!is_remote_url("C:/local"));
        assert!(!is_remote_url(r"\\srv\share"));
        let (p, u, h, port, path) = parse_remote_url("sftp://bob@example.com:2222/home/bob").unwrap();
        assert_eq!(p, Protocol::Sftp);
        assert_eq!(u, "bob");
        assert_eq!(h, "example.com");
        assert_eq!(port, 2222);
        assert_eq!(path, "/home/bob");
    }

    #[test]
    fn remote_endpoint_builds_url() {
        let c = SavedConnection {
            protocol: Protocol::Sftp,
            host: "h".into(),
            port: 22,
            user: "u".into(),
            auth: AuthKind::Password,
            root: "/".into(),
            label: String::new(),
            use_agent: false,
        };
        assert_eq!(remote_endpoint(&c, "/data"), "sftp://u@h:22/data");
    }

    #[test]
    fn norm_root_rules() {
        assert_eq!(norm_root(""), "/");
        assert_eq!(norm_root("home/u"), "/home/u");
        assert_eq!(norm_root("/srv"), "/srv");
        assert_eq!(norm_root("  /x  "), "/x");
    }

    #[test]
    fn enc_userinfo() {
        assert_eq!(enc("user"), "user");
        assert_eq!(enc("a@b:c/d"), "a%40b%3Ac%2Fd");
    }

    #[test]
    fn build_saved_password_and_key() {
        let mut f = ConnectForm::default();
        f.host = "h".into();
        f.user = "u".into();
        f.root = "data".into();
        let s = build_saved(&f, 22);
        assert_eq!(s.protocol, Protocol::Sftp);
        assert_eq!(s.root, "/data");
        assert_eq!(s.auth, AuthKind::Password);

        f.use_key = true;
        f.keyfile = "C:/k".into();
        let s2 = build_saved(&f, 22);
        assert_eq!(s2.auth, AuthKind::Key { path: "C:/k".into() });
    }

    #[test]
    fn from_saved_roundtrips_share() {
        let c = SavedConnection {
            protocol: Protocol::Share,
            host: "srv".into(),
            port: 0,
            user: "bob".into(),
            auth: AuthKind::Password,
            root: r"\\srv\pub".into(),
            label: "Files".into(),
            use_agent: false,
        };
        let f = ConnectForm::from_saved(&c);
        assert_eq!(f.protocol, Protocol::Share);
        assert_eq!(f.unc, r"\\srv\pub");
        assert!(f.save);
    }
}
