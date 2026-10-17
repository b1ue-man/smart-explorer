//! Platform-neutral parts of the vault layer: the error of a locked vault,
//! the unlocked session every vault backend of a site shares, cleartext
//! paths and stage names (C2.6), the listing and capability rules (C1.8,
//! C1.9, C2.5), the locators (C2.2) and the records of the vault list
//! (C2.1). No I/O of their own and no platform code.
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crate::cryptomator::{ContentCryptor, FormatError, Masterkey, MasterkeyCell, VaultConfig};

pub(crate) mod capabilities;
pub(crate) mod listing;
pub(crate) mod locator;
pub(crate) mod path_map;
pub(crate) mod registry_types;

#[cfg(test)]
mod locator_tests;

/// The structured code of a locked vault across process boundaries (IPC
/// `Err.code`, Bridge `ApiError` kind, agent envelope; C3.2).
pub(crate) const VAULT_LOCKED_CODE: &str = "vault_locked";

/// The payload of every error of a locked vault (C3.2). It travels inside an
/// `io::Error` of kind `PermissionDenied`, so `try_exists` (only `NotFound`
/// means absent) never reads a locked vault as empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultLocked {
    /// Site ID of the locked vault (never a path or a name inside it).
    pub(crate) site: String,
}

impl fmt::Display for VaultLocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tresor {} ist gesperrt", self.site)
    }
}

impl std::error::Error for VaultLocked {}

/// The error every operation on a locked vault returns.
pub(crate) fn vault_locked(site: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        VaultLocked {
            site: site.to_string(),
        },
    )
}

/// The site of a `VaultLocked` error; `None` for every other error.
pub(crate) fn locked_site(error: &io::Error) -> Option<&str> {
    error
        .get_ref()?
        .downcast_ref::<VaultLocked>()
        .map(|locked| locked.site.as_str())
}

/// Told once when the session it watches is locked, so it can drop what it
/// derived from the vault's cleartext (K46: memory only, discarded on lock).
pub(crate) trait LockListener: Send + Sync {
    fn locked(&self);
}

/// One unlocked vault: its site, the masterkey and the verified
/// configuration. Every `VaultBackend` of the site shares one session;
/// `lock` wipes the key, tells the listeners, and from then on every backend
/// of the session fails with `VaultLocked` (C3.2). The session register of
/// VLT-M4 creates and locks sessions.
pub(crate) struct VaultSession {
    site: String,
    keys: Arc<MasterkeyCell>,
    config: VaultConfig,
    listeners: Mutex<Vec<Weak<dyn LockListener>>>,
}

impl VaultSession {
    pub(crate) fn new(site: &str, key: Masterkey, config: VaultConfig) -> Arc<Self> {
        Arc::new(Self {
            site: site.to_string(),
            keys: MasterkeyCell::new(key),
            config,
            listeners: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn site(&self) -> &str {
        &self.site
    }

    /// Cipher combination, shortening threshold and vault ID (`jti`).
    pub(crate) fn config(&self) -> &VaultConfig {
        &self.config
    }

    pub(crate) fn keys(&self) -> &Arc<MasterkeyCell> {
        &self.keys
    }

    pub(crate) fn is_locked(&self) -> bool {
        self.keys.is_wiped()
    }

    /// `Err(VaultLocked)` once the vault is locked.
    pub(crate) fn check(&self) -> io::Result<()> {
        if self.is_locked() {
            Err(vault_locked(&self.site))
        } else {
            Ok(())
        }
    }

    /// Wipes the masterkey, then tells every listener still alive.
    pub(crate) fn lock(&self) {
        self.keys.wipe();
        let listeners = std::mem::take(
            &mut *self
                .listeners
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for listener in listeners.iter().filter_map(Weak::upgrade) {
            listener.locked();
        }
    }

    /// Registers `listener` for `lock`; told at once when already locked.
    pub(crate) fn watch(&self, listener: Weak<dyn LockListener>) {
        let mut listeners = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.is_locked() {
            drop(listeners);
            if let Some(listener) = listener.upgrade() {
                listener.locked();
            }
            return;
        }
        listeners.retain(|known| known.strong_count() > 0);
        listeners.push(listener);
    }

    /// Encrypts and decrypts file contents with this session's key.
    pub(crate) fn cryptor(&self) -> ContentCryptor {
        ContentCryptor::new(Arc::clone(&self.keys), self.config.combo)
    }

    /// Runs `use_key` with the masterkey; a wiped key is `VaultLocked`.
    pub(crate) fn with_key<T>(
        &self,
        use_key: impl FnOnce(&Masterkey) -> Result<T, FormatError>,
    ) -> io::Result<T> {
        match self.keys.with(use_key) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) | Err(error) => Err(self.format_error(error)),
        }
    }

    /// A format error as `io::Error`; `Locked` becomes `VaultLocked`.
    pub(crate) fn format_error(&self, error: FormatError) -> io::Error {
        if error == FormatError::Locked {
            vault_locked(&self.site)
        } else {
            error.into()
        }
    }

    /// Any error of an operation that ran into the lock is `VaultLocked`:
    /// the caller sees why, not the inner symptom (a failed tag, a closed
    /// stream).
    pub(crate) fn relock(&self, error: io::Error) -> io::Error {
        if self.is_locked() && locked_site(&error).is_none() {
            vault_locked(&self.site)
        } else {
            error
        }
    }
}

impl fmt::Debug for VaultSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultSession")
            .field("site", &self.site)
            .field("locked", &self.is_locked())
            .finish_non_exhaustive()
    }
}
