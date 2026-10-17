//! Backend keys of the vault locators (`vault://`, `vault-store://`) for the
//! CLI namespace and rename checks in `target.rs`.

/// Keys of the vault locators; `None` for every other spec. `vault://<SITE>`
/// is the cleartext namespace of one site, `vault:<site-id>` for every
/// spelling of `<SITE>`, never the host filesystem: a move between a vault
/// and another place copies, and only paths of one site rename inside it.
/// `vault-store://<site-id>` is the site's storage itself and takes the keys
/// of the site's location, so overlap and rename checks see one folder under
/// both spellings (a namespace of its own without a readable entry). A
/// location is never `vault-store://` (`SiteRecord::validate`), so this
/// recursion ends after one step.
pub(super) fn vault_backend_keys(spec: &str) -> Option<(String, String)> {
    let (scheme, _) = spec.split_once("://")?;
    let key = if scheme.eq_ignore_ascii_case(crate::vault::VAULT_SCHEME) {
        let site = crate::vault::VaultLocator::parse(spec)
            .map(|locator| vault_site_id(&locator.site))
            .unwrap_or_default();
        format!("vault:{site}")
    } else if scheme.eq_ignore_ascii_case(crate::vault::VAULT_STORE_SCHEME) {
        let site = crate::vault::VaultStoreLocator::parse(spec)
            .map(|locator| locator.site)
            .unwrap_or_default();
        if let Ok(Some(record)) = crate::vault::RegistryStore::open_default().get(&site) {
            return Some(super::endpoint_backend_keys(&record.location));
        }
        format!("vault-store:{site}")
    } else {
        return None;
    };
    Some((key.clone(), key))
}

/// The ID of the site `<SITE>` names (ID, ID prefix or label); the spelling
/// itself when the vault list does not resolve it (nothing opens then).
fn vault_site_id(site: &str) -> String {
    let lower = site.to_ascii_lowercase();
    if crate::vault::is_site_id(&lower) {
        return lower;
    }
    crate::vault::RegistryStore::open_default()
        .resolve(site)
        .map_or_else(|_| site.to_string(), |record| record.id)
}

#[cfg(test)]
mod tests {
    /// `se mv` renames only within one namespace (`try_rename_fast`); a
    /// vault path never shares it with a local path, so a move between them
    /// copies instead of renaming inside the vault.
    #[test]
    fn vault_task_cli_move_between_vault_and_local_copies() {
        let id = "0123456789abcdef0123456789abcdef";
        let vault = super::super::endpoint_backend_keys(&format!("vault://{id}/Docs/a.txt"));
        let key = format!("vault:{id}");
        assert_eq!(vault, (key.clone(), key));
        let upper = format!("VAULT://{}/Other", id.to_ascii_uppercase());
        assert_eq!(
            super::super::endpoint_backend_keys(&upper),
            vault,
            "same site"
        );
        for other in [
            "/home/x",
            "C:/tmp/a",
            "gdrive:///a",
            "vault://fedcba9876543210fedcba9876543210/Docs/a.txt",
        ] {
            let keys = super::super::endpoint_backend_keys(other);
            assert_ne!(keys.0, vault.0, "{other}");
            assert_ne!(keys.1, vault.1, "{other}");
        }
    }
}
