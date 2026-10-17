//! Cryptomator vaults as se locations (strand S3 of the plan 2026-10-17,
//! VLT-M2 to VLT-M5):
//!
//! - `VaultBackend`: the cleartext view of an unlocked vault over every se
//!   backend (`vfs::Backend`, scheme `Vault`), on the format core
//!   `crate::cryptomator`.
//! - The locators `vault://<SITE>/<path>` and `vault-store://<site-id>`,
//!   the vault list (`<AD>/vaults/<site-id>.conf`) and their opening through
//!   the shared resolution (`connect::resolve_endpoint`, Android pool).
//!   `vault-store://` resolves without a wrapper to the storage (C2.3).
//! - The session register (VLT-M4): one unlock for every surface of the
//!   device, held by the worker (desktop) or the app process (Android);
//!   locks by hand, idle time, sleep, screen lock, screen off, background
//!   and drive end; local sealing with purpose keys; remembered passwords
//!   only in the secret store of the installation.
//! - Management (VLT-M5): detect, enter, create, remove, password change,
//!   lock settings. Recovery keys are not offered (C1.10).
//!
//! A locked vault answers every operation with `VaultLocked` (code
//! `vault_locked`, C3.2). No cleartext name reaches an error text with an
//! encrypted path, a log or a cache that outlives the unlock.
#![allow(dead_code)] // used by CLI (VLT-M9), GUI (VLT-M10), Android (VLT-M11), drives (VLT-M12)
#![allow(unused_imports)] // the crate API below is re-exported for those consumers

mod core;
mod os;

pub(crate) use self::core::locator::{
    is_site_id, is_vault_locator, vault_location, vault_store_location, VaultLocator,
    VaultStoreLocator, VAULT_SCHEME, VAULT_STORE_SCHEME,
};
pub(crate) use self::core::lock_policy::{
    LockPlatform, LockReason, SignalSupport, SystemSignal, BACKGROUND_CHOICES, DEFAULT_IDLE_MINUTES,
};
pub(crate) use self::core::registry_types::{
    location_key, new_site_id, LockSettings, SiteKind, SiteRecord,
};
pub(crate) use self::core::sealed::{SealPurpose, MAX_SEALED_TEXT};
pub(crate) use self::core::{
    locked_site, vault_locked, LockListener, VaultLocked, VaultSession, VAULT_LOCKED_CODE,
};
pub(crate) use self::os::{
    account, add_site, available_memory, canonical_vault_location, change_password,
    change_site_password, check_new_password, create_site, create_vault, detect_location,
    detect_storage, host_visibility, hosts_register, jobs_using, lock_all_for_exit, open_in,
    open_location_in, open_vault_backend, open_vault_location, open_with, platform_lock_defaults,
    publish_session, recovery_key_info, register, remove_entered, remove_site, resolve_in,
    resolve_store_in, resolve_vault_store, site_locations, unlock_entered, unlock_saved,
    unlock_storage, unlocked_session, update_settings, wire_error_text, withdraw_session,
    ConsumerGuard, ConsumerKind, Detection, Installed, LockError, LockParticipant, LockRequest,
    ManageError, OpenedVault, OsSecretStore, PasswordChanged, PasswordError, RecoveryKeyInfo,
    RegisterEvent, RegistryLoad, RegistryStore, ResolveLocation, SecretStore, SessionRegister,
    SettingsChange, SiteState, SiteStatus, UnlockError, UnlockOptions, Unlocked, VaultBackend,
    VaultOpenError, WorkerOpen, ACCOUNT_PREFIX, CREATE_RECOVERY_HINT, EVENT_LOG, LOCK_PLATFORM,
    MIN_PASSWORD_CHARS, RECOVERY_KEY_NOTE, SCREEN_LOCK_NOTE, VAULTS_DIR,
};
