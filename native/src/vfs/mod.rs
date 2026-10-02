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

#[path = "core/batch.rs"]
mod batch;
#[path = "core/cache.rs"]
mod cache;
#[path = "core/capabilities.rs"]
mod capabilities;
#[path = "core/congestion.rs"]
mod congestion;
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
#[path = "os/shared/local.rs"]
mod local;
#[path = "os/shared/local_extensions.rs"]
mod local_extensions;
#[cfg(windows)]
#[path = "os/windows/local_platform.rs"]
mod local_platform;
#[cfg(not(windows))]
#[path = "os/linux_os/local_platform.rs"]
mod local_platform;
#[path = "core/meta.rs"]
mod meta;
#[path = "core/promotion.rs"]
mod promotion;
#[path = "os/shared/remote_util.rs"]
pub mod remote_util;
#[path = "core/scheme.rs"]
mod scheme;
#[path = "core/staging_names.rs"]
mod staging_names;
#[path = "os/shared/sync_roots.rs"]
mod sync_roots;
pub(crate) use sync_roots::{sync_backend, validate_sync_roots};
#[path = "core/trait_defaults.rs"]
mod trait_defaults;
#[cfg(windows)]
#[path = "os/windows/verbatim.rs"]
mod verbatim;
#[path = "core/volume.rs"]
mod volume;

pub use self::error_classes::{is_target_refusal, omission_reason};
pub use self::extension_calls::{
    change_signal, change_signal_mode, find_duplicates, finish_stage, hash_walk, list_dir_tolerant,
    mtime_precision, open_read_regular, open_write_copy_stage_timed, recycle,
    supports_duplicate_search, supports_hash_walk, supports_recycle, sync_filesystem,
    target_limits, unix_mode, volume_identity,
};
pub use self::extension_types::{
    ChangeNotice, ChangeSignalMode, ChangeSubscription, HashWalkEntry, HashWalkItem,
    HashWalkRequest, MtimePrecision, NameIssue, NameLimit, OmissionReason, RecycleExpectation,
    RecycleOutcome, StageDurability, StageFinish, StageFinished, TargetLimits, VfsListing,
    VfsOmission,
};
pub use self::extensions::BackendExtensions;
pub use self::local_extensions::{local_mount_boundary, local_volume_identity};
pub use self::staging_names::is_staging_name;
pub use self::volume::{MountKind, VolumeIdentity};

pub use self::cache::CachingBackend;
pub use self::capabilities::{MountPathCapabilities, RootConfinement, StagedWriteCapabilities};
pub(crate) use self::copy_transfer::copy_between;
pub use self::core::{
    congestion_error, congestion_of, Backend, BackendHandle, BatchGet, BatchLimits, BatchPut,
    BatchPutOutcome, BatchSink, ChangeKind, Congestion, DedupeCandidate, DeleteDisposition,
    HashHit, Scheme, SearchHit, VfsChange, VfsChangeBatch, VfsMeta, VfsResult,
};
pub(crate) use self::delete::validate_child_name;
pub use self::delete::{
    remove_entry, remove_entry_controlled, DeleteTarget, RecursiveDeleteFailure,
    RecursiveDeletePhase, RecursiveDeleteProgress, RecursiveDeleteReport, RecursiveDeleteStatus,
};
#[allow(unused_imports)]
pub use self::dispatch::{backend_for, is_remote_root};
pub use self::local::LocalBackend;
pub(crate) use self::local_platform::rename_no_replace as promote_local_copy;
pub use self::promotion::{promote_staged_create, promote_staged_replace, unique_staging_path};
pub(crate) use self::promotion::{promote_staged_no_replace_with, promote_staged_with};

#[cfg(test)]
#[path = "os/shared/copy_paste_task_tests.rs"]
mod copy_paste_task_tests;
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
#[path = "core/tests.rs"]
mod tests;
