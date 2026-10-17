//! Cryptomator vaults as se locations (strand S3 of the plan 2026-10-17,
//! VLT-M2 and VLT-M3):
//!
//! - `VaultBackend`: the cleartext view of an unlocked vault over every se
//!   backend (`vfs::Backend`, scheme `Vault`), on the format core
//!   `crate::cryptomator`.
//! - The locators `vault://<SITE>/<path>` and `vault-store://<site-id>`,
//!   the vault list (`<AD>/vaults/<site-id>.conf`) and their opening through
//!   the shared resolution (`connect::resolve_endpoint`, Android pool).
//!   `vault-store://` resolves without a wrapper to the storage (C2.3).
//!
//! A locked vault answers every operation with `VaultLocked` (code
//! `vault_locked`, C3.2). No cleartext name reaches an error text with an
//! encrypted path, a log or a cache that outlives the unlock.
#![allow(dead_code)] // sessions (VLT-M4), CLI (VLT-M9), GUI (VLT-M10) and Android (VLT-M11) use the rest
#![allow(unused_imports)] // the crate API below is re-exported for those consumers

mod core;
mod os;

pub(crate) use self::core::locator::{
    is_site_id, is_vault_locator, vault_location, vault_store_location, VaultLocator,
    VaultStoreLocator, VAULT_SCHEME, VAULT_STORE_SCHEME,
};
pub(crate) use self::core::registry_types::{
    location_key, new_site_id, LockSettings, SiteKind, SiteRecord,
};
pub(crate) use self::core::{
    locked_site, vault_locked, LockListener, VaultLocked, VaultSession, VAULT_LOCKED_CODE,
};
pub(crate) use self::os::{
    canonical_vault_location, open_in, open_vault_backend, open_vault_location, open_with,
    platform_lock_defaults, publish_session, resolve_in, resolve_store_in, resolve_vault_store,
    site_locations, unlocked_session, withdraw_session, OpenedVault, RegistryLoad, RegistryStore,
    ResolveLocation, VaultBackend, VaultOpenError, VAULTS_DIR,
};
