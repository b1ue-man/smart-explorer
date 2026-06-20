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

use super::core::{parse, serialize, SavedConnection};

const KEYRING_SERVICE: &str = "smart_explorer";

fn app_data_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
}

fn connections_path() -> PathBuf {
    app_data_dir().join("connections.txt")
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

fn load_connections_from(path: &std::path::Path) -> Vec<SavedConnection> {
    match std::fs::read_to_string(path) {
        Ok(s) => s.lines().filter_map(parse).collect(),
        Err(_) => Vec::new(),
    }
}

fn save_connections_to(path: &std::path::Path, conns: &[SavedConnection]) -> std::io::Result<()> {
    let body: String = conns.iter().map(serialize).collect::<Vec<_>>().join("\n");
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
    use crate::creds::{AuthKind, Protocol};

    fn sample_pw() -> SavedConnection {
        SavedConnection {
            protocol: Protocol::Sftp,
            host: "example.com".into(),
            port: 2222,
            user: "alice".into(),
            auth: AuthKind::Password,
            root: "/home/alice".into(),
            label: "Work box".into(),
            use_agent: false,
        }
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
