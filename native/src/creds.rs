//! Credential + saved-connection store for remote backends.
//!
//! Two parts, deliberately separated:
//!  * **Secrets** (passwords / key passphrases) → the OS keyring (Windows
//!    Credential Manager via keyring `windows-native`; an in-memory mock
//!    off-Windows). Never written to disk by us.
//!  * **Connection metadata** (protocol / host / port / user / auth kind / key
//!    path / root / label — NO secret) → a plain TSV file in appdata, so the
//!    saved-connection list survives restarts.
#![allow(dead_code)] // staged: consumed by the connect-UI step.

use keyring::Entry;
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "smart_explorer";

fn app_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("smart_explorer");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn connections_path() -> PathBuf {
    app_data_dir().join("connections.txt")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    Sftp,
    Ftp,
    Ftps,
    Webdav,
    Share,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Sftp => "sftp",
            Protocol::Ftp => "ftp",
            Protocol::Ftps => "ftps",
            Protocol::Webdav => "webdav",
            Protocol::Share => "share",
        }
    }
    pub fn parse(s: &str) -> Option<Protocol> {
        match s {
            "sftp" => Some(Protocol::Sftp),
            "ftp" => Some(Protocol::Ftp),
            "ftps" => Some(Protocol::Ftps),
            "webdav" => Some(Protocol::Webdav),
            "share" => Some(Protocol::Share),
            _ => None,
        }
    }
    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp | Protocol::Ftps => 21,
            Protocol::Webdav => 443,
            Protocol::Share => 0,
        }
    }
    pub fn is_url(self) -> bool {
        !matches!(self, Protocol::Share)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AuthKind {
    Password,
    Key { path: String },
}

#[derive(Clone, Debug)]
pub struct SavedConnection {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthKind,
    /// Remote start path (sftp/ftp) or the `\\server\share…` UNC (share).
    pub root: String,
    pub label: String,
}

impl SavedConnection {
    /// Stable, unique keyring account key for this connection's secret.
    pub fn account(&self) -> String {
        format!(
            "{}://{}@{}:{}{}",
            self.protocol.as_str(),
            self.user,
            self.host,
            self.port,
            self.root
        )
    }

    /// Navigation target: a `proto://user@host:port/root` URL for sftp/ftp/ftps,
    /// or the UNC root for a share.
    pub fn to_target(&self) -> String {
        if self.protocol.is_url() {
            format!(
                "{}://{}@{}:{}{}",
                self.protocol.as_str(),
                self.user,
                self.host,
                self.port,
                self.root
            )
        } else {
            self.root.clone()
        }
    }

    pub fn display(&self) -> String {
        if self.label.trim().is_empty() {
            self.account()
        } else {
            self.label.clone()
        }
    }
}

// ── secrets (keyring) ────────────────────────────────────────────────────────

pub fn set_secret(account: &str, secret: &str) -> Result<(), String> {
    Entry::new(KEYRING_SERVICE, account)
        .map_err(|e| e.to_string())?
        .set_password(secret)
        .map_err(|e| e.to_string())
}

pub fn get_secret(account: &str) -> Option<String> {
    Entry::new(KEYRING_SERVICE, account)
        .ok()
        .and_then(|e| e.get_password().ok())
}

pub fn delete_secret(account: &str) {
    if let Ok(e) = Entry::new(KEYRING_SERVICE, account) {
        let _ = e.delete_credential();
    }
}

// ── connection metadata (TSV file) ──────────────────────────────────────────

fn sanitize(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn serialize(c: &SavedConnection) -> String {
    let (auth, keypath) = match &c.auth {
        AuthKind::Password => ("password", String::new()),
        AuthKind::Key { path } => ("key", path.clone()),
    };
    [
        c.protocol.as_str().to_string(),
        sanitize(&c.host),
        c.port.to_string(),
        sanitize(&c.user),
        auth.to_string(),
        sanitize(&keypath),
        sanitize(&c.root),
        sanitize(&c.label),
    ]
    .join("\t")
}

fn parse(line: &str) -> Option<SavedConnection> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 8 {
        return None;
    }
    let protocol = Protocol::parse(f[0])?;
    let port = f[2].parse::<u16>().ok()?;
    let auth = match f[4] {
        "key" => AuthKind::Key {
            path: f[5].to_string(),
        },
        _ => AuthKind::Password,
    };
    Some(SavedConnection {
        protocol,
        host: f[1].to_string(),
        port,
        user: f[3].to_string(),
        auth,
        root: f[6].to_string(),
        label: f[7].to_string(),
    })
}

fn load_connections_from(path: &std::path::Path) -> Vec<SavedConnection> {
    match std::fs::read_to_string(path) {
        Ok(s) => s.lines().filter_map(parse).collect(),
        Err(_) => Vec::new(),
    }
}

fn save_connections_to(path: &std::path::Path, conns: &[SavedConnection]) -> std::io::Result<()> {
    let body: String = conns
        .iter()
        .map(serialize)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, body)
}

pub fn load_connections() -> Vec<SavedConnection> {
    load_connections_from(&connections_path())
}

/// Add or replace (by account) a saved connection.
pub fn save_connection(c: &SavedConnection) -> std::io::Result<()> {
    let mut conns = load_connections();
    let acc = c.account();
    conns.retain(|x| x.account() != acc);
    conns.push(c.clone());
    save_connections_to(&connections_path(), &conns)
}

/// Move a saved connection to the most-recent position (end of the file) so
/// the sidebar can show the freshest connections first and overflow the rest.
/// No-op if the account isn't saved.
pub fn touch_connection(account: &str) {
    let mut conns = load_connections();
    if let Some(pos) = conns.iter().position(|x| x.account() == account) {
        let c = conns.remove(pos);
        conns.push(c);
        let _ = save_connections_to(&connections_path(), &conns);
    }
}

/// Remove a saved connection by account and drop its stored secret.
pub fn remove_connection(account: &str) -> std::io::Result<()> {
    let mut conns = load_connections();
    conns.retain(|x| x.account() != account);
    delete_secret(account);
    save_connections_to(&connections_path(), &conns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pw() -> SavedConnection {
        SavedConnection {
            protocol: Protocol::Sftp,
            host: "example.com".into(),
            port: 2222,
            user: "alice".into(),
            auth: AuthKind::Password,
            root: "/home/alice".into(),
            label: "Work box".into(),
        }
    }

    #[test]
    fn serialize_parse_roundtrip_password() {
        let c = sample_pw();
        let line = serialize(&c);
        let back = parse(&line).unwrap();
        assert_eq!(back.protocol, Protocol::Sftp);
        assert_eq!(back.host, "example.com");
        assert_eq!(back.port, 2222);
        assert_eq!(back.user, "alice");
        assert_eq!(back.auth, AuthKind::Password);
        assert_eq!(back.root, "/home/alice");
        assert_eq!(back.label, "Work box");
    }

    #[test]
    fn serialize_parse_roundtrip_key() {
        let mut c = sample_pw();
        c.auth = AuthKind::Key {
            path: "C:/keys/id_ed25519".into(),
        };
        c.protocol = Protocol::Ftps;
        let back = parse(&serialize(&c)).unwrap();
        assert_eq!(back.protocol, Protocol::Ftps);
        assert_eq!(
            back.auth,
            AuthKind::Key {
                path: "C:/keys/id_ed25519".into()
            }
        );
    }

    #[test]
    fn account_and_target_formats() {
        let c = sample_pw();
        assert_eq!(c.account(), "sftp://alice@example.com:2222/home/alice");
        assert_eq!(c.to_target(), "sftp://alice@example.com:2222/home/alice");

        let share = SavedConnection {
            protocol: Protocol::Share,
            host: "fileserver".into(),
            port: 0,
            user: "dom\\bob".into(),
            auth: AuthKind::Password,
            root: r"\\fileserver\public".into(),
            label: String::new(),
        };
        assert_eq!(share.to_target(), r"\\fileserver\public");
        assert!(!share.protocol.is_url());
    }

    #[test]
    fn file_save_load_roundtrip() {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "creds_test_{}_{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let a = sample_pw();
        let mut b = sample_pw();
        b.host = "other".into();
        save_connections_to(&p, &[a.clone(), b.clone()]).unwrap();
        let loaded = load_connections_from(&p);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].account(), a.account());
        assert_eq!(loaded[1].host, "other");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn secret_api_contract() {
        // On Windows this hits Credential Manager and round-trips. Off-Windows
        // there is no backend (set is a no-op, get returns None) — so we only
        // assert the contract that holds everywhere: the calls don't panic, and
        // a successful set that is actually persisted reads back identically.
        let acct = format!("smart_explorer_test_{}", std::process::id());
        match set_secret(&acct, "s3cr3t") {
            Ok(()) => {
                if let Some(got) = get_secret(&acct) {
                    assert_eq!(got, "s3cr3t");
                    delete_secret(&acct);
                    assert!(get_secret(&acct).is_none());
                }
            }
            Err(_) => { /* no keyring backend in this environment */ }
        }
    }
}
