//! Identity half of the Direct-code renewal (E17): new data next to the
//! current ones, without the `DirectCodeRotated` cleanup marker, so no grant
//! is suspended and the old secret stays until its holders confirmed.
use super::*;

impl ShareIdentity {
    /// The new data in the secure store; nothing durable names them yet.
    /// The returned identity differs from `self` only in lookup id and
    /// Direct secret.
    pub(in crate::share) fn allocate_renewed_with(
        &self,
        storage: &mut impl IdentityPersistence,
    ) -> Result<ShareIdentity, String> {
        let (direct_lookup_id, direct_secret) =
            allocate_direct_secret(storage, Some(&self.direct_lookup_id))?;
        let mut candidate = self.clone();
        candidate.direct_lookup_id = direct_lookup_id;
        candidate.direct_secret = direct_secret;
        Ok(candidate)
    }

    /// Makes the allocated data current (identity file, no cleanup marker).
    pub(in crate::share) fn commit_renewed_with(
        &self,
        storage: &mut impl IdentityPersistence,
    ) -> Result<(), String> {
        self.save_with(storage)
    }

    /// The secret a retiring lookup id still verifies with.
    pub(in crate::share) fn retiring_secret_with(
        storage: &mut impl IdentityPersistence,
        lookup_id: &str,
    ) -> Result<Option<[u8; SECRET_BYTES]>, String> {
        load_optional_secret(
            storage,
            &direct_secret_account(lookup_id),
            "Auslaufender Direkt-Code",
        )
    }

    /// Deletes the secret of a lookup id nobody waits for any more.
    pub(in crate::share) fn delete_retiring_secret_with(
        storage: &mut impl IdentityPersistence,
        lookup_id: &str,
    ) -> Result<(), String> {
        storage
            .delete_secret(&direct_secret_account(lookup_id))
            .map_err(|error| format!("Auslaufenden Direkt-Code entfernen: {error}"))
    }

    /// Whether a renewal runs and how many devices it still waits for
    /// (every surface shows this, plan M22).
    pub fn code_renewal_status(&self) -> code_renewal::CodeRenewalStatus {
        code_renewal_registry::status(&self.node_id)
    }
}
