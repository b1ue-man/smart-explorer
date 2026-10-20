//! Host-facing parts of the vault layer: the vault backend over every se
//! backend, its directory cache, the vault list on disk and the opening of
//! `vault://` and `vault-store://` locations (VLT-M2, VLT-M3); the session
//! register, unlocking, remembered passwords, management and password
//! change (VLT-M4, VLT-M5); the vault side of vault drives: sealed spool,
//! spool keys, sealed journal fields and the bridge to the drive manager
//! (VLT-M12). The shared parts are portable (`shared/`); the lock signals
//! and the memory reading of each platform live in `windows.rs`,
//! `linux_os.rs` and `android.rs`, the thumbnail cleanup of vault drives in
//! `linux_thumbnails.rs` (Linux only).
use std::path::Path;

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
// U26 (VLT-M8): the comparison mode of vault sync jobs and „Tresor prüfen“.
#[path = "shared/compare.rs"]
mod compare;
#[path = "shared/compare_actions.rs"]
mod compare_actions;
#[path = "shared/compare_cache.rs"]
mod compare_cache;
#[path = "shared/compare_engine.rs"]
mod compare_engine;
#[path = "shared/compare_files.rs"]
mod compare_files;
#[path = "shared/compare_folders.rs"]
mod compare_folders;
#[path = "shared/compare_items.rs"]
mod compare_items;
#[path = "shared/compare_keep_both.rs"]
mod compare_keep_both;
#[path = "shared/compare_merge.rs"]
mod compare_merge;
#[path = "shared/dir_cache.rs"]
mod dir_cache;
#[path = "shared/dir_map.rs"]
mod dir_map;
// U20 (VLT-M6, VLT-M7): memory spool, places, replicas and vault sync jobs.
#[path = "shared/ephemeral_spool.rs"]
mod ephemeral_spool;
#[path = "shared/health.rs"]
mod health;
#[path = "shared/health_fixes.rs"]
mod health_fixes;
#[path = "shared/journal_sealer.rs"]
mod journal_sealer;
#[path = "shared/local_store.rs"]
mod local_store;
#[path = "shared/manage.rs"]
mod manage;
#[path = "shared/mount_bridge.rs"]
mod mount_bridge;
#[path = "shared/open.rs"]
mod open;
#[path = "shared/password.rs"]
mod password;
#[path = "shared/registry_store.rs"]
mod registry_store;
#[path = "shared/remember.rs"]
mod remember;
#[path = "shared/replica.rs"]
mod replica;
#[path = "shared/sealed_spool.rs"]
mod sealed_spool;
#[path = "shared/session.rs"]
mod session;
#[path = "shared/spool_key.rs"]
mod spool_key;
#[path = "shared/sync_job.rs"]
mod sync_job;
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

// Thumbnails of vault drive folders (K45, VLT-M12): freedesktop cache.
#[cfg(target_os = "linux")]
#[path = "linux_thumbnails.rs"]
mod thumbnails;

pub(crate) use backend::VaultBackend;
pub(crate) use compare::{compare_job, compare_job_in, compare_snapshot, find_vault_job};
pub(crate) use compare_actions::{
    resolve_item, resolve_item_in, ActionEnv, ResolveReport, ResolveRequest,
};
pub(crate) use compare_cache::{is_stale as is_compare_stale, stale as compare_stale};
pub(crate) use compare_merge::{compare_text_job, merge_apply_item, MERGE_MAX_BYTES};
pub(crate) use ephemeral_spool::EphemeralSpool;
pub(crate) use health::{check_site, check_site_in, CheckResult};
pub(crate) use journal_sealer::{sealed_mount_storage, JournalPathSealer};
pub(crate) use local_store::{
    expand_dir_sort, expand_places, note_vault_backend, publish_vault_mount, seal_log_fields,
    shows_vault, store_dir_sort, store_places, vault_mount_of, vault_mount_roots, vault_site_of,
    withdraw_vault_mount, LocalStore, PlaceList, ProcessSealing, SiteSealing, VaultPlaces,
};
pub(crate) use manage::{
    add_site, create_site, create_vault, detect_location, detect_storage, jobs_using,
    recovery_key_info, remove_entered, remove_site, update_settings, Detection, ManageError,
    RecoveryKeyInfo, SettingsChange, CREATE_RECOVERY_HINT, RECOVERY_KEY_NOTE,
};
pub(crate) use mount_bridge::{
    vault_drive_bridge, MountVaultError, Resolver, UnlockHook, VaultDriveBridge,
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
pub(crate) use replica::{
    create_vault_job, default_replicas_root, CopyTarget, JobPlace, VaultJobCreated,
};
pub(crate) use sealed_spool::{SealedSpoolFile, SealedSpoolStorage};
pub(crate) use session::{
    host_visibility, hosts_register, lock_all_for_exit, register, wire_error_text, ConsumerGuard,
    ConsumerKind, Installed, LockError, LockParticipant, LockRequest, RegisterEvent,
    RegisterOptions, SessionRegister, SiteState, SiteStatus, EVENT_LOG,
};
pub(crate) use spool_key::{SpoolKey, SPOOL_KEY_FILE};
pub(crate) use sync_job::{
    apply_vault_preview_action, preview_vault_job_in, run_vault_job, run_vault_job_in,
    VaultPreview, VaultRunOutcome, VaultRunRequest,
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

/// Removes the freedesktop thumbnails of files inside the drive folder
/// `place` (K45, S2-V7); `root` stands for the user's thumbnail cache. Only
/// Linux keeps such a cache; elsewhere nothing is removed. Returns how many.
pub(crate) fn remove_drive_thumbnails(root: Option<&Path>, place: &Path) -> usize {
    #[cfg(target_os = "linux")]
    {
        let root = root.map(Path::to_path_buf).or_else(thumbnails::cache_root);
        root.map_or(0, |root| thumbnails::remove_for_place(&root, place))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, place);
        0
    }
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
// U26 (VLT-M8): the comparison mode, its actions and the check.
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
#[path = "shared/compare_actions_tests.rs"]
mod compare_actions_tests;
#[cfg(test)]
#[path = "shared/compare_folders_tests.rs"]
mod compare_folders_tests;
#[cfg(test)]
#[path = "shared/compare_test_support.rs"]
mod compare_test_support;
#[cfg(test)]
#[path = "shared/compare_tests.rs"]
mod compare_tests;
#[cfg(test)]
#[path = "shared/dir_map_tests.rs"]
mod dir_map_tests;
#[cfg(test)]
#[path = "shared/health_tests.rs"]
mod health_tests;
#[cfg(test)]
#[path = "shared/interop_tests.rs"]
mod interop_tests;
#[cfg(test)]
#[path = "shared/manage_tests.rs"]
mod manage_tests;
#[cfg(test)]
#[path = "shared/mount_bridge_tests.rs"]
mod mount_bridge_tests;
#[cfg(test)]
#[path = "shared/password_tests.rs"]
mod password_tests;
#[cfg(test)]
#[path = "shared/registry_tests.rs"]
mod registry_tests;
#[cfg(test)]
#[path = "shared/sealed_spool_tests.rs"]
mod sealed_spool_tests;
#[cfg(test)]
#[path = "shared/session_lock_tests.rs"]
mod session_lock_tests;
#[cfg(test)]
#[path = "shared/session_test_support.rs"]
mod session_test_support;
#[cfg(test)]
#[path = "shared/session_tests.rs"]
mod session_tests;
#[cfg(test)]
#[path = "shared/spool_key_tests.rs"]
mod spool_key_tests;
