//! Host-facing parts of the vault layer: the vault backend over every se
//! backend, its directory cache, the vault list on disk and the opening of
//! `vault://` and `vault-store://` locations (VLT-M2, VLT-M3); the session
//! register, unlocking, remembered passwords, management and password
//! change (VLT-M4, VLT-M5). The shared parts are portable (`shared/`); the
//! lock signals and the memory reading of each platform live in
//! `windows.rs`, `linux_os.rs` and `android.rs`.
use crate::vault::core::lock_policy::LockPlatform;
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
#[path = "shared/manage.rs"]
mod manage;
#[path = "shared/open.rs"]
mod open;
#[path = "shared/password.rs"]
mod password;
#[path = "shared/registry_store.rs"]
mod registry_store;
#[path = "shared/remember.rs"]
mod remember;
#[path = "shared/session.rs"]
mod session;
#[path = "shared/unlock.rs"]
mod unlock;

// Lock signals and memory of the platform (C3.4, C1.3 (c)).
#[cfg(target_os = "android")]
#[path = "android.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

pub(crate) use backend::VaultBackend;
pub(crate) use manage::{
    add_site, create_site, create_vault, detect_location, detect_storage, jobs_using,
    recovery_key_info, remove_entered, remove_site, update_settings, Detection, ManageError,
    RecoveryKeyInfo, SettingsChange, CREATE_RECOVERY_HINT, RECOVERY_KEY_NOTE,
};
pub(crate) use open::{
    canonical_vault_location, open_in, open_location_in, open_vault_backend, open_vault_location,
    open_with, publish_session, resolve_store_in, resolve_vault_store, site_locations,
    unlocked_session, withdraw_session, OpenedVault, ResolveLocation, VaultOpenError, WorkerOpen,
};
pub(crate) use password::{
    change_password, change_site_password, check_new_password, PasswordChanged, PasswordError,
    MIN_PASSWORD_CHARS,
};
pub(crate) use registry_store::{resolve_in, RegistryLoad, RegistryStore, VAULTS_DIR};
pub(crate) use remember::{account, OsSecretStore, SecretStore, ACCOUNT_PREFIX};
pub(crate) use session::{
    host_visibility, hosts_register, lock_all_for_exit, register, wire_error_text, ConsumerGuard,
    ConsumerKind, Installed, LockError, LockParticipant, LockRequest, RegisterEvent,
    RegisterOptions, SessionRegister, SiteState, SiteStatus, EVENT_LOG,
};
pub(crate) use unlock::{
    read_key_files, unlock_entered, unlock_saved, unlock_storage, KeyFiles, UnlockError,
    UnlockOptions, Unlocked,
};

/// The lock triggers of this platform (C3.3, C7.1).
#[cfg(target_os = "android")]
pub(crate) const LOCK_PLATFORM: LockPlatform = LockPlatform::Android;
/// The lock triggers of this platform (C3.3, C7.1).
#[cfg(not(target_os = "android"))]
pub(crate) const LOCK_PLATFORM: LockPlatform = LockPlatform::Desktop;

/// What surfaces say next to the screen lock switch (C2 S3 K13).
pub(crate) const SCREEN_LOCK_NOTE: Option<&str> = platform::SCREEN_LOCK_NOTE;

/// Memory scrypt may take in this process (C1.3 (c)); `None` = unknown,
/// then only the fixed limits apply.
pub(crate) fn available_memory() -> Option<u64> {
    platform::available_memory()
}

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
#[path = "shared/interop_tests.rs"]
mod interop_tests;
#[cfg(test)]
#[path = "shared/manage_tests.rs"]
mod manage_tests;
#[cfg(test)]
#[path = "shared/password_tests.rs"]
mod password_tests;
#[cfg(test)]
#[path = "shared/registry_tests.rs"]
mod registry_tests;
#[cfg(test)]
#[path = "shared/session_lock_tests.rs"]
mod session_lock_tests;
#[cfg(test)]
#[path = "shared/session_test_support.rs"]
mod session_test_support;
#[cfg(test)]
#[path = "shared/session_tests.rs"]
mod session_tests;
