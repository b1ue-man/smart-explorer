//! Virtual filesystem layer - the single, standardized interface Smart Explorer
//! talks to, on top of which every storage backend is built: local disk today,
//! SFTP / FTP / network drives next, cloud later. See
//! `docs/REMOTE_LAYER_PLAN.md`.
//!
//! Design (verified):
//!  * **The trait is BLOCKING.** The whole app is synchronous (rayon +
//!    `std::thread` + crossbeam). Remote backends own a private runtime and
//!    `block_on` internally, so scanner / copy / UI never see async.
//!  * **Paths are FORWARD-SLASH strings.** The app already stores paths that
//!    way; each backend converts to its own convention at the boundary.
//!  * **Self-contained.** This module adds no edits to the hot local scan/copy
//!    loops. `LocalBackend` mirrors today's `std::fs` behavior so the remote
//!    scan/copy paths added with the SFTP/FTP backends (and any later
//!    unification) can route through ONE interface without putting a vtable in
//!    the hot local walk. The local fast path stays exactly as it is.
#![allow(dead_code)] // staged interface: wired in by the SFTP/FTP/connect steps.

#[path = "core/backend_defaults.rs"]
mod backend_defaults;
#[path = "core/batch.rs"]
mod batch;
#[path = "core/cache.rs"]
mod cache;
#[path = "core/capacity.rs"]
mod capacity;
pub use self::capacity::{
    capacity, capacity_failure, observed_now_ms, CapacityFailure, CapacitySnapshot, CapacitySource,
    CapacityState,
};
#[path = "core/capabilities.rs"]
mod capabilities;
#[path = "core/congestion.rs"]
mod congestion;
#[path = "core/content_digest.rs"]
mod content_digest;
#[path = "core/peer_namespace.rs"]
mod peer_namespace;
pub use peer_namespace::{
    is_synthetic_container, normalize as normalize_share_path, split_clean as split_share_path,
    CONNECTIONS_ROOT as SHARE_CONNECTIONS_ROOT,
};
#[path = "core/sync_signature.rs"]
mod sync_signature;
pub use self::content_digest::{content_digest, ContentDigest};
pub use self::content_digest::{
    digest_by_local_read, is_md5_hex, is_sha256_hex, md5_exact, sha256_exact,
};
pub use sync_signature::{is_engine_name, Sig, REPLICA_MARKER_NAME, VERSIONS_DIR_NAME};
#[path = "os/shared/copy_transfer.rs"]
mod copy_transfer;
#[path = "core/core.rs"]
mod core;
#[path = "core/dedupe.rs"]
mod dedupe;
#[path = "core/delete.rs"]
mod delete;
#[path = "core/dispatch.rs"]
mod dispatch;
#[path = "core/error_classes.rs"]
mod error_classes;
#[path = "core/extension_calls.rs"]
mod extension_calls;
#[path = "core/extension_types.rs"]
mod extension_types;
#[path = "core/extensions.rs"]
mod extensions;
#[path = "core/file_identity.rs"]
mod file_identity;
#[cfg(windows)]
#[path = "os/windows/capacity.rs"]
mod local_capacity;
#[cfg(not(windows))]
#[path = "os/linux_os/capacity.rs"]
mod local_capacity;
pub use file_identity::file_identity;
#[path = "core/folder_reservation.rs"]
mod folder_reservation;
pub use folder_reservation::{
    create_dir_exclusive, directory_identity, directory_reservation_supported,
    rename_reserved_directory,
};
#[path = "os/shared/confined_local.rs"]
mod confined_local;
#[path = "os/shared/confined_local_extensions.rs"]
mod confined_local_extensions;
#[cfg(windows)]
#[path = "os/windows/confined_local.rs"]
mod confined_local_platform;
#[cfg(not(windows))]
#[path = "os/linux_os/confined_local.rs"]
mod confined_local_platform;
pub use confined_local::ConfinedLocalBackend;
#[path = "core/fs_profile.rs"]
mod fs_profile;
#[path = "core/key_tally.rs"]
mod key_tally;
#[path = "os/shared/local.rs"]
mod local;
#[path = "os/shared/local_dirs.rs"]
mod local_dirs;
#[path = "os/shared/local_extensions.rs"]
mod local_extensions;
#[path = "os/shared/local_file_identity.rs"]
mod local_file_identity;
pub use local_file_identity::local_file_identity_handle;
#[cfg(windows)]
#[path = "os/windows/local_platform.rs"]
mod local_platform;
#[cfg(not(windows))]
#[path = "os/linux_os/local_platform.rs"]
mod local_platform;
#[path = "os/shared/local_stage.rs"]
mod local_stage;
#[path = "core/location_paths.rs"]
pub mod location_paths;
#[path = "core/meta.rs"]
mod meta;
#[path = "core/ports.rs"]
pub mod ports;
#[path = "core/promotion.rs"]
mod promotion;
#[path = "core/ranges.rs"]
mod ranges;
#[path = "os/shared/remote_util.rs"]
pub mod remote_util;
#[path = "core/scheme.rs"]
mod scheme;
#[path = "core/staging_names.rs"]
mod staging_names;
#[path = "os/shared/sync_roots.rs"]
mod sync_roots;
pub use sync_roots::{sync_backend, validate_sync_roots};
#[path = "core/trait_defaults.rs"]
mod trait_defaults;
#[path = "core/transfer_evidence.rs"]
mod transfer_evidence;
#[path = "core/tree_remove_listed.rs"]
mod tree_remove_listed;
#[path = "core/tree_remove_pipelined.rs"]
mod tree_remove_pipelined;
#[path = "core/unpublished.rs"]
mod unpublished;
#[cfg(windows)]
#[path = "os/windows/verbatim.rs"]
mod verbatim;
#[path = "core/volume.rs"]
mod volume;

pub use self::error_classes::{is_target_refusal, omission_reason};
pub use self::extension_calls::confirm_namespace;
pub use self::extension_calls::default_sync_child_path as literal_child_path;
pub use self::extension_calls::{
    change_signal, change_signal_mode, content_md5, content_md5_many, find_duplicates,
    finish_stage, hash_walk, hash_walk_keyed, list_dir_tolerant, mtime_precision,
    open_read_regular, open_write_copy_stage_timed, previous_state_identities, recycle,
    remove_tree, replace_staged_reversible, supports_duplicate_search, supports_hash_walk,
    supports_recycle, supports_remove_tree, sync_child_path, sync_filesystem, sync_path, sync_stat,
    target_limits, unix_mode, version_archive_policy, volume_identity,
};
pub use self::extension_types::{
    ChangeNotice, ChangeSignalMode, ChangeSubscription, HashWalkEntry, HashWalkItem, HashWalkMeta,
    HashWalkRequest, KeyedWalk, KeyedWalkItem, MtimePrecision, NameIssue, NameLimit,
    OmissionReason, RecycleExpectation, RecycleOutcome, StageDurability, StageFinish,
    StageFinished, TargetLimits, VfsListing, VfsOmission,
};
pub use self::extensions::{BackendExtensions, VersionArchivePolicy};
pub use self::key_tally::{name_is_unusual, KeyTally};
pub use self::local_extensions::{
    local_mount_boundary, local_volume_identity, LocalMountBoundaries,
};
pub use self::staging_names::is_staging_name;
pub use self::staging_names::{fit_stage_name, is_unique_stage, split_stage_name};
pub use self::transfer_evidence::{
    batch_put_times, create_dirs, host_batch_limits, open_read_evidence, open_write_absent,
    open_write_absent_timed, open_write_fresh_timed, promote_copy_stage_evidence,
    put_batch_evidence, server_copy_batch, single_put, supports_create_dirs, take_stage_evidence,
    transfer_evidence, transfer_scope, EvidencePutOutcome, FileEvidence, PreparePut, StageEvidence,
};
pub use self::tree_remove_listed::{remove_tree_listed, TREE_LEAF_PORTION};
pub use self::tree_remove_pipelined::remove_leaves_pipelined;
pub use self::volume::{MountKind, VolumeIdentity};

pub use self::cache::CachingBackend;
pub use self::capabilities::{MountPathCapabilities, RootConfinement, StagedWriteCapabilities};
pub use self::copy_transfer::copy_between;
pub use self::core::{
    congestion_error, congestion_of, Backend, BackendHandle, BatchGet, BatchLimits, BatchPut,
    BatchPutOutcome, BatchSink, ChangeKind, Congestion, DedupeCandidate, DeleteDisposition,
    HashHit, Scheme, SearchHit, VfsChange, VfsChangeBatch, VfsMeta, VfsResult,
};
pub use self::delete::validate_child_name;
pub use self::delete::{
    remove_entry, remove_entry_controlled, DeleteTarget, RecursiveDeleteFailure,
    RecursiveDeletePhase, RecursiveDeleteProgress, RecursiveDeleteReport, RecursiveDeleteStatus,
};
#[allow(unused_imports)]
pub use self::dispatch::{backend_for, is_remote_root};
pub use self::local::LocalBackend;
/// Windows: refuses a new name Win32 would read as a stream or reject (the
/// copy's own stages and handle renames check with it, A7).
#[cfg(windows)]
pub use self::local_platform::check_new_name as check_local_new_name;
pub use self::local_platform::create_new_private as create_local_copy_stage;
pub use self::local_platform::rename_no_replace as promote_local_copy;
pub use self::local_platform::replace_file as replace_local_file;
pub use self::ports::{
    validate_sync_endpoints, LocalServerCopy, SealedSpool, SealedSpools, SyncEndpointRules,
    UrlBackends, VAULT_SIDE_REFUSED,
};
pub use self::promotion::{
    creates_stages_exclusively, promote_staged_create, promote_staged_replace, stage_proven,
    unique_staging_path,
};
pub use self::promotion::{promote_staged_no_replace_with, promote_staged_with};
pub use self::ranges::SegmentedStage;
pub use self::unpublished::{unpublished_error, unpublished_of};

#[cfg(test)]
#[path = "core/content_digest_task_tests.rs"]
mod content_digest_task_tests;
#[cfg(test)]
#[path = "os/shared/copy_paste_task_tests.rs"]
mod copy_paste_task_tests;
#[cfg(test)]
#[path = "core/delete_speed_task_tests.rs"]
mod delete_speed_task_tests;
#[cfg(test)]
#[path = "core/delete_tests.rs"]
mod delete_tests;
#[cfg(test)]
#[path = "core/promotion_tests.rs"]
mod promotion_tests;
#[cfg(test)]
#[path = "core/remote_drive_task_cache_tests.rs"]
mod remote_drive_task_cache_tests;
#[cfg(test)]
#[path = "core/review_task_contract_tests.rs"]
mod review_task_contract_tests;
#[cfg(test)]
#[path = "os/shared/review_task_local_tests.rs"]
mod review_task_local_tests;
#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
