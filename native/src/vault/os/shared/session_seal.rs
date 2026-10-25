//! Local sealing with the purpose keys of an unlocked site (C3.6): the
//! worker seals for itself and, over IPC `Seal`/`Unseal`, for processes
//! without the key (GUI, CLI). A purpose key exists only inside one call;
//! the spool wrap key never leaves the worker. A locked site answers
//! `VaultLocked`.
use std::io;

use zeroize::Zeroizing;

use crate::cryptomator::FormatError;
use crate::vault::core::sealed::{spool_wrap_key, SealPurpose, Sealer};
use crate::vault::core::vault_locked;

use super::SessionRegister;

impl SessionRegister {
    /// Runs `use_sealer` with the sealer of `purpose` of the unlocked `site`.
    pub(crate) fn with_sealer<T>(
        &self,
        site: &str,
        purpose: SealPurpose,
        use_sealer: impl FnOnce(&Sealer) -> Result<T, FormatError>,
    ) -> io::Result<T> {
        let (session, _) = self.session(site).ok_or_else(|| vault_locked(site))?;
        if matches!(purpose, SealPurpose::TransferJob | SealPurpose::TransferAlias) {
            let record = crate::vault::RegistryStore::open_default().resolve(site).map_err(|_| vault_locked(site))?;
            if record.vault_id != session.config().jti { return Err(vault_locked(site)); }
        }
        session.with_key(|key| {
            let sealer = Sealer::new(key, purpose)?;
            use_sealer(&sealer)
        })
    }

    /// Each item sealed as Base64url text.
    pub(crate) fn seal_texts<S: AsRef<str>>(
        &self,
        site: &str,
        purpose: SealPurpose,
        items: &[S],
    ) -> io::Result<Vec<String>> {
        self.with_sealer(site, purpose, |sealer| {
            items
                .iter()
                .map(|item| sealer.seal_text(item.as_ref()))
                .collect()
        })
    }

    /// The texts of `seal_texts` results; one changed item fails the call.
    pub(crate) fn unseal_texts<S: AsRef<str>>(
        &self,
        site: &str,
        purpose: SealPurpose,
        items: &[S],
    ) -> io::Result<Vec<Zeroizing<String>>> {
        self.with_sealer(site, purpose, |sealer| {
            items
                .iter()
                .map(|item| sealer.unseal_text(item.as_ref()))
                .collect()
        })
    }

    /// Runs `use_key` with the spool wrap key of the unlocked `site`
    /// (VLT-M12 wraps the spool keys of the vault's drives with it).
    pub(crate) fn with_spool_wrap_key<T>(
        &self,
        site: &str,
        use_key: impl FnOnce(&[u8; 32]) -> T,
    ) -> io::Result<T> {
        let (session, _) = self.session(site).ok_or_else(|| vault_locked(site))?;
        session.with_key(|key| spool_wrap_key(key).map(|wrap| use_key(&wrap)))
    }
}
