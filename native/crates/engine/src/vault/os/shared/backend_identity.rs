//! Caller-supplied provider IDs never bypass the vault's mapped storage path.
use super::backend::VaultBackend;
use std::io;

impl VaultBackend {
    pub(super) fn checked_storage_id(
        &self,
        path: &str,
        supplied: Option<&str>,
    ) -> io::Result<Option<String>> {
        let Some(id) = supplied.filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        let expected = self.inner.item_id(path)?;
        match expected {
            Some(expected) if expected == id => Ok(Some(expected)),
            Some(_) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Objekt-ID gehört nicht zum gewählten Eintrag im Tresor",
            )),
            // A backend without object identities is addressed by the mapped
            // pathname; an opaque caller ID grants no additional capability.
            None => Ok(None),
        }
    }
}
