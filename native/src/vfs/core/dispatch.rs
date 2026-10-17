use std::io;
use std::sync::Arc;

use super::{BackendHandle, LocalBackend};

/// Locators of vaults (`vault://` cleartext, `vault-store://` ciphertext):
/// they name a registered vault site, never a path, and open only through
/// `connect::resolve_endpoint`.
const VAULT_PREFIXES: [&str; 2] = ["vault://", "vault-store://"];

/// Build the backend for a root string. Remote schemes are recognized by URL
/// prefix; everything else (drive paths, `\\server\share` UNC, mapped drives)
/// is local. The SFTP/FTP/SMB arms are filled in by their respective steps so
/// that adding a protocol never touches the callers. An `smb://` root needs its
/// password in the URL; saved connections open through `connect`. Vault
/// locators are refused: they must never reach the local backend.
pub fn backend_for(root: &str) -> io::Result<BackendHandle> {
    let r = root.trim();
    let lower = r.to_ascii_lowercase();
    if is_vault_locator(&lower) {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Tresororte (vault://, vault-store://) öffnet nur die Ortsauflösung",
        ))
    } else if lower.starts_with("sftp://") {
        Ok(Arc::new(crate::sftp::backend_from_url(r)?))
    } else if lower.starts_with("ftp://") || lower.starts_with("ftps://") {
        Ok(Arc::new(crate::ftp::backend_from_url(r)?))
    } else if lower.starts_with("smb://") {
        Ok(Arc::new(crate::smb::backend_from_url(r)?))
    } else {
        Ok(Arc::new(LocalBackend::new(r)))
    }
}

/// Whether a root string is served by a remote (non-local) backend. Lets the
/// app pick the remote scan path and disable the inotify watcher for remote
/// roots without constructing a backend. Vault locators are never local.
pub fn is_remote_root(root: &str) -> bool {
    let lower = root.trim().to_ascii_lowercase();
    lower.starts_with("sftp://")
        || lower.starts_with("ftp://")
        || lower.starts_with("ftps://")
        || lower.starts_with("smb://")
        || is_vault_locator(&lower)
}

fn is_vault_locator(lower: &str) -> bool {
    VAULT_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}
