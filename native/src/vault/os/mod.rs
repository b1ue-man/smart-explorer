//! Host-facing parts of the vault layer: the vault backend over every se
//! backend, its directory cache, the vault list on disk and the opening of
//! `vault://` and `vault-store://` locations. All of it is portable
//! (`shared/`); the platform lock defaults are chosen here.
use crate::vault::core::registry_types::LockSettings;

#[path = "shared/backend.rs"]
mod backend;
#[path = "shared/backend_copy.rs"]
mod backend_copy;
#[path = "shared/backend_dirs.rs"]
mod backend_dirs;
#[path = "shared/backend_extensions.rs"]
mod backend_extensions;
#[path = "shared/backend_io.rs"]
mod backend_io;
#[path = "shared/backend_listing.rs"]
mod backend_listing;
#[path = "shared/backend_nodes.rs"]
mod backend_nodes;
#[path = "shared/backend_rename.rs"]
mod backend_rename;
#[path = "shared/backend_write.rs"]
mod backend_write;
#[path = "shared/dir_cache.rs"]
mod dir_cache;
#[path = "shared/open.rs"]
mod open;
#[path = "shared/registry_store.rs"]
mod registry_store;

pub(crate) use backend::VaultBackend;
pub(crate) use open::{
    canonical_vault_location, open_in, open_vault_backend, open_vault_location, open_with,
    publish_session, resolve_store_in, resolve_vault_store, site_locations, unlocked_session,
    withdraw_session, OpenedVault, ResolveLocation, VaultOpenError,
};
pub(crate) use registry_store::{resolve_in, RegistryLoad, RegistryStore, VAULTS_DIR};

/// Lock defaults of sites on this platform (C3.3).
#[cfg(target_os = "android")]
pub(crate) fn platform_lock_defaults() -> LockSettings {
    LockSettings::android()
}

/// Lock defaults of sites on this platform (C3.3).
#[cfg(not(target_os = "android"))]
pub(crate) fn platform_lock_defaults() -> LockSettings {
    LockSettings::desktop()
}

#[cfg(test)]
#[path = "shared/backend_dirs_tests.rs"]
mod backend_dirs_tests;
#[cfg(test)]
#[path = "shared/backend_test_support.rs"]
mod backend_test_support;
#[cfg(test)]
#[path = "shared/backend_tests.rs"]
mod backend_tests;
#[cfg(test)]
#[path = "shared/backend_write_tests.rs"]
mod backend_write_tests;
#[cfg(test)]
#[path = "shared/registry_tests.rs"]
mod registry_tests;
