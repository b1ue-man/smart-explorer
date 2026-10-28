//! Remote-drive mounting boundaries.

#[path = "core/baseline_match.rs"]
mod baseline_match;
#[path = "core/cache_policy.rs"]
mod cache_policy;
#[path = "core/cache_space.rs"]
mod cache_space;
#[path = "core/case_semantics.rs"]
mod case_semantics;
#[path = "core/clean_cache.rs"]
mod clean_cache;
#[path = "core/commit.rs"]
mod commit;
#[path = "core/delete.rs"]
mod delete;
#[path = "core/delete_recovery.rs"]
mod delete_recovery;
// DRV-M6: the device tree „Verbundene Geräte als Ordner“.
#[path = "core/device_tree.rs"]
mod device_tree;
#[path = "core/device_tree_backend.rs"]
mod device_tree_backend;
#[path = "core/device_tree_children.rs"]
mod device_tree_children;
#[path = "core/device_tree_fs.rs"]
mod device_tree_fs;
#[path = "core/device_tree_names.rs"]
mod device_tree_names;
#[cfg(test)]
#[path = "core/device_tree_names_tests.rs"]
mod device_tree_names_tests;
#[path = "core/device_tree_routing.rs"]
mod device_tree_routing;
#[path = "core/device_tree_types.rs"]
mod device_tree_types;
// DRV-M6: `PeerOpenTarget` to `PeerMountTarget` and the key of a device.
#[path = "core/bulk_copy.rs"]
mod bulk_copy;
#[path = "os/shared/device_key.rs"]
mod device_key;
#[path = "core/engine.rs"]
mod engine;
#[path = "core/engine_recovery.rs"]
mod engine_recovery;
#[path = "core/entry_lifecycle.rs"]
mod entry_lifecycle;
#[path = "core/entry_table.rs"]
mod entry_table;
#[path = "core/file_commit.rs"]
mod file_commit;
#[path = "core/file_commit_stage.rs"]
mod file_commit_stage;
#[path = "core/file_io.rs"]
mod file_io;
#[path = "core/frontend.rs"]
mod frontend;
#[path = "core/frontend_engine.rs"]
mod frontend_engine;
#[cfg(test)]
#[path = "core/frontend_tests.rs"]
mod frontend_tests;
#[path = "core/hydration.rs"]
mod hydration;
#[path = "core/journal.rs"]
mod journal;
#[path = "core/journal_record.rs"]
mod journal_record;
#[path = "core/materialization.rs"]
mod materialization;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/metadata_batch.rs"]
mod metadata_batch;
#[path = "core/metadata_cache.rs"]
mod metadata_cache;
#[path = "core/metadata_loading.rs"]
mod metadata_loading;
#[path = "core/metadata_point_cache.rs"]
mod metadata_point_cache;
#[path = "core/metadata_policy.rs"]
mod metadata_policy;
#[path = "core/mount_source.rs"]
mod mount_source;
#[path = "core/mount_status.rs"]
mod mount_status;
#[path = "core/mutations.rs"]
mod mutations;
#[path = "core/namespace_recovery.rs"]
mod namespace_recovery;
#[path = "core/open_handle.rs"]
mod open_handle;
#[path = "core/path.rs"]
mod path;
#[path = "core/path_dialect.rs"]
mod path_dialect;
#[cfg(test)]
#[path = "core/path_dialect_tests.rs"]
mod path_dialect_tests;
#[path = "core/path_sealer.rs"]
mod path_sealer;
#[path = "core/peer_names.rs"]
pub(crate) mod peer_names;
#[path = "core/peer_roots.rs"]
mod peer_roots;
#[path = "core/pending_attributes.rs"]
mod pending_attributes;
#[path = "core/range_read.rs"]
mod range_read;
#[path = "core/recovery.rs"]
mod recovery;
#[path = "core/recovery_export.rs"]
mod recovery_export;
#[path = "os/shared/recovery_export.rs"]
mod recovery_export_io;
#[path = "core/recovery_identity.rs"]
mod recovery_identity;
#[path = "os/shared/recovery_identity.rs"]
mod recovery_identity_io;
#[path = "core/recovery_import.rs"]
mod recovery_import;
#[path = "os/shared/recovery_import.rs"]
mod recovery_import_io;
#[path = "core/recovery_manifest.rs"]
mod recovery_manifest;
#[path = "core/recovery_state.rs"]
mod recovery_state;
#[cfg(windows)]
#[path = "os/windows/recovery_storage.rs"]
mod recovery_storage;
#[cfg(not(windows))]
#[path = "os/linux_os/recovery_storage.rs"]
mod recovery_storage;
#[path = "core/replace.rs"]
mod replace;
#[path = "core/retirement_queue.rs"]
mod retirement_queue;
#[path = "core/spool.rs"]
mod spool;
#[path = "core/spool_storage.rs"]
mod spool_storage;
#[cfg(test)]
#[path = "core/spool_storage_tests.rs"]
mod spool_storage_tests;
#[path = "core/stage_ledger.rs"]
mod stage_ledger;
#[path = "core/startup.rs"]
mod startup;
#[path = "core/types.rs"]
mod types;
#[cfg(test)]
#[path = "core/types_split_tests.rs"]
mod types_split_tests;
#[path = "core/upload_queue.rs"]
mod upload_queue;
#[cfg(test)]
#[path = "core/vault_scheduler_task_tests.rs"]
mod vault_scheduler_task_tests;
#[path = "core/windows_access.rs"]
mod windows_access;
#[path = "core/windows_case.rs"]
mod windows_case;
#[path = "core/write_back.rs"]
mod write_back;

#[cfg(all(test, not(windows)))]
#[path = "core/metadata_cache_task_tests.rs"]
mod metadata_cache_task_tests;

#[cfg(all(test, not(windows)))]
#[path = "core/metadata_snapshot_task_tests.rs"]
mod metadata_snapshot_task_tests;

#[cfg(all(test, not(windows)))]
#[path = "core/navigation_cache_task_tests.rs"]
mod navigation_cache_task_tests;

#[cfg(all(test, not(windows)))]
#[path = "core/remote_drive_task_tests.rs"]
mod remote_drive_task_tests;

pub(crate) mod os;

pub(crate) use metadata_cache::MetadataChange;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) use os::linux_os::CacheLease as RecoveryCacheLease;
#[cfg(windows)]
pub(crate) use os::windows::CacheLease as RecoveryCacheLease;
pub(crate) use recovery_export_io::{export_mount_recovery, recovery_cache_ids};
pub(crate) use recovery_identity::{
    conflict as recovery_identity_conflict, manifest_identity as recovery_manifest_identity,
    missing as recovery_identity_missing, RecoveryBackendIdentity,
};
pub(crate) use recovery_identity_io::{
    bind_mount_recovery_identity, preflight_mount_recovery_identity, read_mount_recovery_identity,
};
pub(crate) use recovery_import_io::{
    confirm_recovery_namespace, copy_recovery_package, read_recovery_manifest,
    restore_recovery_cache, verify_recovery_package,
};
pub use recovery_manifest::{
    InactiveMountRecovery, RecoveryFile, RecoveryGap, RecoveryGapKind, RecoveryJournal,
    RecoveryManifest, RECOVERY_FORMAT_VERSION,
};
#[cfg(test)]
pub(crate) use recovery_state::fail_next_recovery_at;
pub(crate) use recovery_state::{recovery_checkpoint, RecoveryCommitPoint};
pub(crate) use recovery_storage::publish as publish_recovery_object;

#[cfg(all(test, unix))]
#[path = "core/drive_speed_task_tests.rs"]
mod drive_speed_task_tests;
#[cfg(test)]
#[path = "core/mount_save_task_tests.rs"]
mod mount_save_task_tests;
#[cfg(test)]
#[path = "core/optimization_cache_tests.rs"]
mod optimization_cache_tests;
#[cfg(test)]
#[path = "core/optimization_fixture.rs"]
pub(crate) mod optimization_fixture;
#[cfg(test)]
#[path = "core/optimization_metadata_tests.rs"]
mod optimization_metadata_tests;
#[cfg(test)]
#[path = "core/optimization_policy_tests.rs"]
mod optimization_policy_tests;

#[cfg(test)]
#[path = "core/range_read_task_tests.rs"]
pub(crate) mod range_read_task_tests;
#[cfg(test)]
#[path = "core/recovery_cache_task_tests.rs"]
mod recovery_cache_task_tests;

pub use cache_policy::{
    MountCachePolicy, MountRuntimePreference, DEFAULT_MOUNT_CACHE_MIB, MAX_MOUNT_CACHE_MIB,
};
pub use cache_space::CacheSpaceProbe;
pub use device_key::{device_key, mount_target_of, open_target_of};
pub use device_tree::DeviceTree;
pub use device_tree_children::pending_devices;
pub use device_tree_names::{
    device_key_of, folder_names, layout, room_folder_names, same_name, Entry as DeviceEntry,
    Layout as DeviceLayout, RoomFolder, DEVICES_MOUNT_ID, ROOMS_FOLDER,
};
pub use device_tree_types::{
    ChildConnection, ChildLink, ChildRoot, DeviceKind, DeviceRow, RoomRef, TreeConfig,
};
pub use engine::MountEngine;
pub use frontend::{
    LockReadiness, MountFilesystem, PendingAttributes, PreloadProgress, StopProbe, VolumeFacts,
};
pub use metadata_policy::{
    MountMetadataPolicy, DEFAULT_METADATA_PRELOAD_DEPTH, MAX_METADATA_PRELOAD_DEPTH,
};
pub use path::{PathProjector, ProjectedPath};
pub use path_dialect::PathDialect;
pub use path_sealer::{PathSealer, PlainPathSealer};
pub use peer_roots::{
    list_peer_roots, peer_root_choices, safe_virtual_segment, PeerRootChoice, PeerRootListing,
    PEER_CONNECTIONS_ROOT,
};
pub use recovery_state::{MountRecovery, MountSnapshot};
pub use spool::prepare_spool_root;
pub use spool_storage::{MountStorage, PlainSpoolStorage, SpoolFile, SpoolStorage};
pub use types::{
    BackendRoot, Baseline, DeleteToken, DriveLetter, DriveSelection, EntryCondition, FlushOutcome,
    HandleId, MountConfig, MountConflict, MountId, MountLocation, MountMode, MountRootSecurity,
    MountRuntimeConfig, MountSource, MountStatus, NamespaceOutcome, OpenDisposition,
    OpenFileOptions, PeerMountTarget, RenameOutcome,
};
pub use upload_queue::MountUploadStatus;
// DRV-M4: the Linux runtime report, its install answer and the unmount from
// outside the host (`se drive`, the desktop window).
#[cfg(target_os = "linux")]
pub use os::linux_os::{
    fuse_install_outcome, fuse_runtime_report, unmount_drive_folder, FuseInstallOutcome,
    FuseRuntimeReport,
};
#[cfg_attr(not(windows), allow(unused_imports))]
pub(crate) use windows_access::{
    maximum_allowed_full_grant, maximum_allowed_read_grant, requests_maximum_allowed,
};
pub(crate) use windows_case::{validate_windows_case_component, windows_ordinal_key};

#[path = "core/runtime_types.rs"]
mod runtime_types;
pub(crate) use runtime_types::{
    validate_dokany_version_domains, DokanyVersionCompatibilityError,
    DOKANY_DRIVER_PROTOCOL_VERSION, DOKANY_LIBRARY_API_VERSION,
};
pub use runtime_types::{DriveRuntimeInfo, DriveRuntimeInstallOutcome};
#[path = "os/shared/runtime_info.rs"]
mod runtime_info;
pub use runtime_info::{
    devices_label, drive_mount_supported, drive_runtime_info, drive_runtime_ready,
    install_drive_runtime,
};

/// Recognizes the exact private invocation used by the daemon to start the
/// isolated filesystem host (Windows: Dokany, Linux: FUSE, DRV-M4).
/// Authentication still requires the one-use capability environment created
/// by the daemon. A host that ended on an update-stop request (contract V8)
/// leaves the process here with `EXIT_UPDATE_STOP` (75), so no supervisor
/// takes it for a crash.
pub fn run_host_if_requested(arguments: &[std::ffi::OsString]) -> Option<Result<(), String>> {
    if arguments.len() != 2 || arguments[0] != std::ffi::OsStr::new("--mount-host") {
        return None;
    }
    let id = arguments[1]
        .to_str()
        .ok_or_else(|| "mount host id is not valid Unicode".to_string())
        .and_then(|value| MountId::parse(value).map_err(|error| error.to_string()));
    #[cfg(windows)]
    return Some(
        id.and_then(os::windows::run_mount_host)
            .map(exit_on_update_stop),
    );
    #[cfg(target_os = "linux")]
    return Some(
        id.and_then(os::linux_os::run_mount_host)
            .map(exit_on_update_stop),
    );
    #[cfg(not(any(windows, target_os = "linux")))]
    Some(id.and_then(|_| {
        Err("virtual drive hosts are supported only on Windows and Linux".to_string())
    }))
}

#[cfg(any(windows, target_os = "linux"))]
fn exit_on_update_stop(exit: os::shared::host_session::HostExit) {
    if exit == os::shared::host_session::HostExit::UpdateStop {
        std::process::exit(exit.exit_code());
    }
}

#[path = "core/capacity.rs"]
mod capacity;
pub use capacity::{MountCapacity, MountCapacityReport};
#[path = "os/shared/capacity_refresh.rs"]
mod capacity_refresh;
