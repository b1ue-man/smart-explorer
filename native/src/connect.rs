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
                    ConnectResult::Ok(Connected {
                        remote: Some(RemoteState {
                            backend: Arc::new(be),
                            label: label.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let f = ConnectForm::from_saved(&c);
        assert_eq!(f.protocol, Protocol::Share);
        assert_eq!(f.unc, r"\\srv\pub");
        assert!(f.save);
    }
}
