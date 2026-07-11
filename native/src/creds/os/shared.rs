//! Credential + saved-connection store for remote backends.
//!
//! Two parts, deliberately separated:
//!  * **Secrets** (passwords / key passphrases) → the platform credential
//!    backend. Windows uses Credential Manager; Linux uses bounded,
//!    owner-protected files so headless sessions do not depend on D-Bus.
//!  * **Connection metadata** (protocol / host / port / user / auth kind / key
//!    path / root / label — NO secret) → a plain TSV file in appdata, so the
//!    saved-connection list survives restarts.
#![allow(dead_code)] // staged: consumed by the connect-UI step.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::core::{parse, serialize, SavedConnection};
use super::{secure_store, transaction};

static STORE_WRITE_LOCK: Mutex<()> = Mutex::new(());
const MAX_CONNECTION_BYTES: u64 = 1024 * 1024;
const STORE_LOCK_FILE: &str = "connections.lock";

struct StoreWriteGuard {
    _process_guard: MutexGuard<'static, ()>,
    _file_guard: File,
}

fn app_data_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
}

fn connections_path() -> PathBuf {
    app_data_dir().join("connections.txt")
}

// ── secrets (platform credential backend) ───────────────────────────────────

pub fn set_secret(account: &str, secret: &str) -> Result<(), String> {
    secure_store::set_secret(account, secret)
}

pub fn get_secret_checked(account: &str) -> Result<Option<String>, String> {
    secure_store::get_secret(account)
}

/// Compatibility helper for read paths that already treat a missing or
/// inaccessible credential as unavailable. Mutating flows use the checked API.
pub fn get_secret(account: &str) -> Option<String> {
    get_secret_checked(account).ok().flatten()
}

pub fn delete_secret_checked(account: &str) -> Result<(), String> {
    secure_store::delete_secret(account)
}

/// Compatibility helper for non-critical cleanup. Removal and disconnect
/// flows use `delete_secret_checked` so they cannot report false success.
pub fn delete_secret(account: &str) {
    let _ = delete_secret_checked(account);
}

/// Human-readable backend identity for diagnostics. This intentionally says
/// nothing about individual accounts and never reads secret material.
pub fn secret_store_description() -> &'static str {
    secure_store::description()
}

/// Verify that the selected backend can be opened without creating a secret.
pub fn probe_secret_store() -> Result<(), String> {
    secure_store::get_secret("smart-explorer:credential-store-probe")
        .map(|_| ())
        .map_err(|error| format!("Anmeldeinformationsspeicher pruefen: {error}"))
}

// ── connection metadata (TSV file) ──────────────────────────────────────────

fn load_connections_from(path: &Path) -> Vec<SavedConnection> {
    match read_connections_file(path) {
        Ok(s) => s.lines().filter_map(parse).collect(),
        Err(_) => Vec::new(),
    }
}

fn load_connections_for_update(path: &Path) -> std::io::Result<Vec<SavedConnection>> {
    let body = match read_connections_file(path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    body.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            parse(line).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Ungültige gespeicherte Verbindung in Zeile {}", index + 1),
                )
            })
        })
        .collect()
}

fn read_connections_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "connection metadata is not a regular file",
        ));
    }
    if metadata.len() > MAX_CONNECTION_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "connection metadata exceeds its 1 MiB limit",
        ));
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "connection metadata length does not fit this platform",
        )
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    Read::by_ref(&mut file)
        .take(MAX_CONNECTION_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONNECTION_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "connection metadata exceeds its 1 MiB limit",
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "connection metadata is not valid UTF-8",
        )
    })
}

fn save_connections_to(path: &Path, conns: &[SavedConnection]) -> std::io::Result<()> {
    let body: String = conns.iter().map(serialize).collect::<Vec<_>>().join("\n");
    if body.len() as u64 > MAX_CONNECTION_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "connection metadata exceeds its 1 MiB limit",
        ));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(body.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

fn store_write_guard() -> std::io::Result<StoreWriteGuard> {
    let process_guard = match STORE_WRITE_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let directory = app_data_dir();
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(STORE_LOCK_FILE);
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if !metadata.file_type().is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "connection transaction lock is not a regular file",
            ));
        }
    }
    let file_guard = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file_guard.lock()?;
    Ok(StoreWriteGuard {
        _process_guard: process_guard,
        _file_guard: file_guard,
    })
}

fn restore_secret(account: &str, previous: Option<&str>) -> Result<(), String> {
    match previous {
        Some(secret) => set_secret(account, secret),
        None => delete_secret_checked(account),
    }
}

pub fn load_connections() -> Vec<SavedConnection> {
    load_connections_from(&connections_path())
}

/// Strict connection metadata load for CLI/automation paths. Unlike the
/// compatibility helper above, corruption and I/O failures remain visible.
pub fn load_connections_checked() -> Result<Vec<SavedConnection>, String> {
    load_connections_for_update(&connections_path())
        .map_err(|error| format!("Verbindungsmetadaten lesen: {error}"))
}

/// Add or replace (by account) a saved connection.
pub fn save_connection(c: &SavedConnection) -> std::io::Result<()> {
    let _guard = store_write_guard()?;
    let path = connections_path();
    let mut conns = load_connections_for_update(&path)?;
    let acc = c.account();
    conns.retain(|x| x.account() != acc);
    conns.push(c.clone());
    save_connections_to(&path, &conns)
}

/// Add or replace connection metadata together with its optional secret.
/// A metadata failure restores the previous credential before returning.
pub fn save_connection_with_secret(
    c: &SavedConnection,
    secret: Option<&str>,
) -> Result<(), String> {
    let _guard =
        store_write_guard().map_err(|error| format!("Verbindungsspeicher sperren: {error}"))?;
    let path = connections_path();
    let mut conns = load_connections_for_update(&path)
        .map_err(|error| format!("Verbindungsmetadaten lesen: {error}"))?;
    let account = c.account();
    conns.retain(|item| item.account() != account);
    conns.push(c.clone());

    let Some(secret) = secret.filter(|secret| !secret.is_empty()) else {
        return save_connections_to(&path, &conns)
            .map_err(|error| format!("Verbindungsmetadaten speichern: {error}"));
    };
    let previous = get_secret_checked(&account)
        .map_err(|error| format!("Vorherige Anmeldeinformation lesen: {error}"))?;

    transaction::commit_secret_and_metadata(
        "Verbindung speichern",
        || set_secret(&account, secret),
        || {
            save_connections_to(&path, &conns)
                .map_err(|error| format!("Verbindungsmetadaten speichern: {error}"))
        },
        || restore_secret(&account, previous.as_deref()),
    )
}

/// Move a saved connection to the most-recent position (end of the file) so
/// the sidebar can show the freshest connections first and overflow the rest.
/// No-op if the account isn't saved.
pub fn touch_connection(account: &str) -> std::io::Result<()> {
    let _guard = store_write_guard()?;
    touch_connection_in(&connections_path(), account)
}

fn touch_connection_in(path: &Path, account: &str) -> std::io::Result<()> {
    // MRU updates are mutations too: use the strict reader so a malformed or
    // temporarily unreadable store can never be rewritten as a partial list.
    let mut conns = load_connections_for_update(path)?;
    if let Some(pos) = conns.iter().position(|x| x.account() == account) {
        let c = conns.remove(pos);
        conns.push(c);
        save_connections_to(path, &conns)?;
    }
    Ok(())
}

/// Remove a saved connection by account and drop its stored secret.
pub fn remove_connection(account: &str) -> Result<(), String> {
    let _guard =
        store_write_guard().map_err(|error| format!("Verbindungsspeicher sperren: {error}"))?;
    let path = connections_path();
    let mut conns = load_connections_for_update(&path)
        .map_err(|error| format!("Verbindungsmetadaten lesen: {error}"))?;
    conns.retain(|x| x.account() != account);
    let previous = get_secret_checked(account)
        .map_err(|error| format!("Anmeldeinformation vor dem Entfernen lesen: {error}"))?;

    transaction::commit_secret_and_metadata(
        "Verbindung entfernen",
        || delete_secret_checked(account),
        || {
            save_connections_to(&path, &conns)
                .map_err(|error| format!("Verbindungsmetadaten speichern: {error}"))
        },
        || restore_secret(account, previous.as_deref()),
    )
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

    #[cfg(windows)]
    #[test]
    fn windows_secret_api_contract() {
        let acct = format!("smart_explorer_test_{}", std::process::id());
        set_secret(&acct, "s3cr3t").unwrap();
        assert_eq!(
            get_secret_checked(&acct).unwrap().as_deref(),
            Some("s3cr3t")
        );
        delete_secret_checked(&acct).unwrap();
        assert!(get_secret_checked(&acct).unwrap().is_none());
    }

    #[test]
    fn update_rejects_malformed_metadata_instead_of_overwriting_it() {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "creds_invalid_test_{}_{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&p, "not-a-valid-connection").unwrap();
        let error = load_connections_for_update(&p).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "not-a-valid-connection"
        );
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn touch_rejects_malformed_metadata_without_erasing_it() {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "creds_touch_invalid_test_{}_{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let original = format!("{}\nnot-a-valid-connection", serialize(&sample_pw()));
        std::fs::write(&p, &original).unwrap();

        let error = touch_connection_in(&p, &sample_pw().account()).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
        std::fs::remove_file(&p).ok();
    }
}
