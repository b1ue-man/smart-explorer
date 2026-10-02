//! Safe two-way (and one-way) sync between two `vfs::Backend`s.
//!
//! Safety is the whole point ("it just works" — the default must be safe):
//!  * A **baseline** from the previous run records each side's state, so we know
//!    which side actually CHANGED — not just which differs. One side changed →
//!    propagate. BOTH sides changed a file → it's a **conflict**, surfaced for
//!    the user; never silently overwritten (strict file-level default).
//!  * Every overwrite/delete is **reversible**: the old bytes are copied into a
//!    versions store first, pruned by a retention window — so any sync action
//!    can be undone.
//!  * `dry_run` reports the plan without touching anything.
//!
//! Backend-agnostic (local↔local, local↔SFTP, …). The line-level git-style
//! merge is a future optional mode; the shipped default is the strict
//! file-level one the spec asks for.
#![allow(dead_code)] // engine; the sync UI wiring lands next.
#![allow(unused_imports)] // re-exports below preserve the crate::bisync API surface.

#[path = "os/shared/apply.rs"]
mod apply;
#[path = "os/shared/apply_delete.rs"]
mod apply_delete;
#[path = "os/shared/apply_groups.rs"]
mod apply_groups;
#[path = "os/shared/apply_guard.rs"]
mod apply_guard;
#[path = "os/shared/apply_pool.rs"]
mod apply_pool;
#[path = "os/shared/apply_retry.rs"]
mod apply_retry;
#[path = "os/shared/apply_transfer.rs"]
mod apply_transfer;
#[path = "os/shared/checkpoint.rs"]
mod checkpoint;
#[path = "core/completion.rs"]
mod completion;
#[path = "core/plan.rs"]
mod core;
#[path = "os/shared/duplicate_apply.rs"]
mod duplicate_apply;
#[path = "os/shared/duplicate_backup.rs"]
mod duplicate_backup;
#[path = "os/shared/duplicate_observation.rs"]
mod duplicate_observation;
#[path = "os/shared/duplicate_plan.rs"]
mod duplicate_plan;
#[path = "core/duplicate_types.rs"]
mod duplicate_types;
#[path = "os/shared/incremental.rs"]
mod incremental;
#[path = "os/shared/incremental_changes.rs"]
mod incremental_changes;
#[path = "os/shared/incremental_collect.rs"]
mod incremental_collect;
#[path = "core/keys.rs"]
mod keys;
#[path = "core/limits.rs"]
mod limits;
#[path = "os/shared/move_finalize.rs"]
mod move_finalize;
#[path = "core/omissions.rs"]
mod omissions;
#[path = "os/shared/orchestration.rs"]
mod orchestration;
#[path = "os/shared/orchestration_full.rs"]
mod orchestration_full;
#[path = "os/shared/pair_lock.rs"]
mod pair_lock;
#[path = "core/paths.rs"]
mod paths;
#[path = "os/shared/persistence.rs"]
mod persistence;
#[path = "os/shared/preview.rs"]
mod preview;
#[path = "os/shared/replica_state.rs"]
mod replica_state;
#[path = "os/shared/resolve.rs"]
mod resolve_conflict;
#[path = "core/run_types.rs"]
mod run_types;
#[path = "os/shared/snapshot.rs"]
mod snapshot;
#[path = "os/shared/snapshot_agent.rs"]
mod snapshot_agent;
#[path = "os/shared/snapshot_dir.rs"]
mod snapshot_dir;
#[path = "os/shared/snapshot_duplicates.rs"]
mod snapshot_duplicates;
#[path = "os/shared/snapshot_hash.rs"]
mod snapshot_hash;
#[path = "os/shared/snapshot_pair.rs"]
mod snapshot_pair;
#[path = "core/snapshot_types.rs"]
mod snapshot_types;
#[path = "os/shared/snapshot_walk.rs"]
mod snapshot_walk;
#[path = "os/shared/state_store.rs"]
mod state_store;
#[path = "os/shared/state_types.rs"]
mod state_types;
#[path = "os/shared/state_validation.rs"]
mod state_validation;
#[path = "os/shared/sync_flows.rs"]
pub(crate) mod sync_flows;
#[path = "os/shared/sync_overload.rs"]
pub(crate) mod sync_overload;
#[path = "core/types.rs"]
mod types;
#[path = "os/shared/versions.rs"]
pub mod versions;

pub use apply::apply;
pub use completion::{ApplySink, CompletedAction, CompletedKind, DirAction};
pub use core::{plan, update_baseline};
pub use duplicate_types::{DuplicateConflict, FileVariant};
pub use keys::{KeyPolicy, Spellings};
pub use limits::SyncLimits;
pub use omissions::{OmissionKind, SyncOmissions};
pub use orchestration::{pair_key_policy, run, run_with, Outcome, RunRequest};
pub use pair_lock::{pair_lock_id, PairLock};
pub use paths::{is_engine_name, REPLICA_MARKER_NAME, VERSIONS_DIR_NAME};
pub use persistence::{
    baseline_path, load_baseline, pair_id, pair_id_for, prune_versions, save_baseline, versions_dir,
};
pub use preview::{apply_preview_action, preview, Preview};
pub use replica_state::{
    baseline_file, forget_job_state, forget_pair_state, merge_baseline_entries,
};
pub use resolve_conflict::{
    resolve, resolve_checked, resolve_recorded, resolve_variant_checked, ResolvePhase,
};
pub use run_types::{
    BlockConfirmation, ReplicaRef, RunBlock, RunSettings, RunStop, ScanDepth, StateKey, StateOwner,
};
pub use snapshot::{empty_globset, walk_files, HashMode, WalkFilter};
pub use snapshot_types::{DirSet, SideSnapshot};
pub use types::{
    Action, Baseline, BisyncOptions, BisyncStats, CompareMode, Conflict, ConflictMode,
    DeletePolicy, Direction, PairSide, Sig, Throttle, Tree, Versioning, VersioningScheme,
    VersionsLocation,
};

#[cfg(test)]
#[path = "core/contract_tests.rs"]
mod contract_tests;
#[cfg(all(test, windows))]
#[path = "os/windows/link_fixture.rs"]
pub(crate) mod link_fixture;
#[cfg(all(test, unix))]
#[path = "os/linux_os/link_fixture.rs"]
pub(crate) mod link_fixture;
#[cfg(test)]
#[path = "os/shared/test_remote.rs"]
pub(crate) mod test_remote;
#[cfg(test)]
#[path = "os/shared/tests.rs"]
mod tests;
