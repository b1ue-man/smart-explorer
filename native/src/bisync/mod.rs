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
#[path = "os/shared/apply_actions.rs"]
mod apply_actions;
#[path = "os/shared/apply_boundary.rs"]
pub(crate) mod apply_boundary;
#[path = "os/shared/apply_dedupe.rs"]
mod apply_dedupe;
#[path = "os/shared/apply_delete.rs"]
mod apply_delete;
#[path = "os/shared/apply_dirs.rs"]
mod apply_dirs;
#[path = "os/shared/apply_groups.rs"]
mod apply_groups;
#[path = "os/shared/apply_guard.rs"]
mod apply_guard;
#[path = "os/shared/apply_mirror.rs"]
mod apply_mirror;
#[path = "os/shared/apply_pool.rs"]
mod apply_pool;
#[path = "os/shared/apply_reporting.rs"]
mod apply_reporting;
#[path = "os/shared/apply_retry.rs"]
mod apply_retry;
#[path = "os/shared/apply_stage.rs"]
pub(crate) mod apply_stage;
#[path = "os/shared/apply_transaction.rs"]
pub(crate) mod apply_transaction;
#[path = "os/shared/apply_transfer.rs"]
mod apply_transfer;
#[path = "os/shared/backend_identity_migration.rs"]
mod backend_identity_migration;
#[path = "os/shared/backend_identity_state.rs"]
mod backend_identity_state;
#[path = "core/baseline_records.rs"]
mod baseline_records;
#[path = "os/shared/checkpoint.rs"]
mod checkpoint;
#[path = "os/shared/checkpoint_journal.rs"]
mod checkpoint_journal;
#[path = "os/shared/checkpoint_run.rs"]
mod checkpoint_run;
#[path = "core/compare.rs"]
mod compare;
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
#[path = "os/shared/engine_change_feed.rs"]
mod engine_change_feed;
#[cfg(test)]
#[path = "os/shared/engine_provider_task_tests.rs"]
mod engine_provider_task_tests;
#[path = "core/guards.rs"]
mod guards;
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
#[path = "os/shared/merge_execution.rs"]
mod merge_execution;
#[path = "os/shared/merge_inputs.rs"]
mod merge_inputs;
#[path = "os/shared/merge_keep_both.rs"]
mod merge_keep_both;
#[path = "os/shared/merge_precheck.rs"]
mod merge_precheck;
#[path = "os/shared/merge_recorded.rs"]
mod merge_recorded;
#[path = "os/shared/merge_recovery.rs"]
mod merge_recovery;
#[path = "os/shared/merge_resume.rs"]
mod merge_resume;
#[cfg(test)]
#[path = "os/shared/merge_task_fixture.rs"]
mod merge_task_fixture;
#[path = "os/shared/move_finalize.rs"]
mod move_finalize;
#[path = "core/omissions.rs"]
mod omissions;
#[path = "os/shared/orchestration.rs"]
mod orchestration;
#[path = "os/shared/orchestration_full.rs"]
mod orchestration_full;
#[path = "os/shared/orchestration_plan.rs"]
mod orchestration_plan;
#[path = "os/shared/pair_lock.rs"]
mod pair_lock;
#[path = "core/paths.rs"]
mod paths;
#[path = "os/shared/persistence.rs"]
mod persistence;
#[path = "os/shared/persistence_versions.rs"]
mod persistence_versions;
#[path = "core/plan_decide.rs"]
mod plan_decide;
#[path = "core/plan_dirs.rs"]
mod plan_dirs;
#[path = "core/plan_filter.rs"]
mod plan_filter;
#[path = "core/plan_index.rs"]
mod plan_index;
#[path = "core/plan_pair.rs"]
mod plan_pair;
#[path = "core/plan_types.rs"]
mod plan_types;
#[path = "os/shared/preview.rs"]
mod preview;
#[path = "os/shared/recorded_paths.rs"]
mod recorded_paths;
#[path = "os/shared/replacement_journal.rs"]
mod replacement_journal;
#[path = "os/shared/replacement_publish.rs"]
mod replacement_publish;
#[path = "os/shared/replacement_recovery.rs"]
mod replacement_recovery;
#[path = "os/shared/replica.rs"]
mod replica;
#[path = "os/shared/replica_state.rs"]
mod replica_state;
#[path = "os/shared/resolve.rs"]
mod resolve_conflict;
#[path = "core/run_types.rs"]
mod run_types;
#[path = "os/shared/single_recorded.rs"]
mod single_recorded;
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
#[path = "os/shared/snapshot_mounts.rs"]
mod snapshot_mounts;
#[path = "os/shared/snapshot_pair.rs"]
mod snapshot_pair;
#[path = "os/shared/snapshot_policy.rs"]
pub(crate) mod snapshot_policy;
#[path = "core/snapshot_types.rs"]
mod snapshot_types;
#[path = "os/shared/snapshot_walk.rs"]
mod snapshot_walk;
#[path = "os/shared/state_bootstrap.rs"]
mod state_bootstrap;
#[path = "os/shared/state_metadata.rs"]
mod state_metadata;
#[path = "os/shared/state_spellings.rs"]
mod state_spellings;
#[path = "os/shared/state_store.rs"]
mod state_store;
#[path = "os/shared/state_types.rs"]
mod state_types;
#[path = "os/shared/state_validation.rs"]
mod state_validation;
#[path = "os/shared/sync_flows.rs"]
pub(crate) mod sync_flows;
#[path = "os/shared/sync_observation.rs"]
mod sync_observation;
#[path = "os/shared/sync_overload.rs"]
pub(crate) mod sync_overload;
#[path = "os/shared/transfer_stream.rs"]
pub(crate) mod transfer_stream;
#[path = "core/types.rs"]
mod types;
#[path = "os/shared/version_listing.rs"]
mod version_listing;
#[path = "os/shared/version_manifest.rs"]
mod version_manifest;
#[path = "os/shared/version_ops.rs"]
mod version_ops;
#[path = "os/shared/version_restore.rs"]
mod version_restore;
#[path = "os/shared/version_retention.rs"]
mod version_retention;
#[path = "os/shared/version_save.rs"]
mod version_save;
#[path = "os/shared/versions.rs"]
pub mod versions;

pub use apply::apply;
pub use completion::{ApplySink, CompletedAction, CompletedKind, DirAction};
pub use core::{plan, update_baseline};
pub use duplicate_types::{DuplicateConflict, FileVariant};
pub use keys::{KeyPolicy, Spellings};
pub use limits::SyncLimits;
pub use merge_inputs::{PendingMerge, RecordedMergeChoice};
pub use merge_recorded::{
    merge_recorded_for_key, MergeChoice, MergeFailure, MergeFile, MergeReport, OriginalContent,
};
pub use merge_resume::{pending_merge_for_key, pending_merge_relatives};
pub use omissions::{OmissionKind, SyncOmissions};
pub use orchestration::{pair_key_policy, run, run_with, Outcome, RunRequest};
pub use pair_lock::{pair_lock_id, PairLock};
pub use paths::{is_engine_name, REPLICA_MARKER_NAME, VERSIONS_DIR_NAME};
pub use persistence::{
    baseline_path, load_baseline, pair_id, pair_id_for, prune_versions, save_baseline, versions_dir,
};
pub use preview::{apply_preview_action, preview, preview_with, Preview};
pub use recorded_paths::{recorded_original_paths_for_key, RecordedPaths};
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
pub(crate) use snapshot_hash::hash_file as current_content_signature;
pub use snapshot_types::{DirSet, SideSnapshot};
pub use types::{
    Action, Baseline, BisyncOptions, BisyncStats, CompareMode, Conflict, ConflictMode,
    DeletePolicy, Direction, PairSide, Sig, Throttle, Tree, Versioning, VersioningScheme,
    VersionsLocation,
};

#[cfg(test)]
#[path = "os/shared/checkpoint_review_tests.rs"]
mod checkpoint_review_tests;
#[cfg(test)]
#[path = "core/contract_tests.rs"]
mod contract_tests;
#[cfg(test)]
#[path = "os/shared/index_review_tests.rs"]
mod index_review_tests;
#[cfg(all(test, windows))]
#[path = "os/windows/link_fixture.rs"]
pub(crate) mod link_fixture;
#[cfg(all(test, unix))]
#[path = "os/linux_os/link_fixture.rs"]
pub(crate) mod link_fixture;
#[cfg(test)]
#[path = "core/plan_review_tests.rs"]
mod plan_review_tests;
#[cfg(test)]
#[path = "os/shared/replica_review_tests.rs"]
mod replica_review_tests;
#[cfg(test)]
#[path = "os/shared/test_remote.rs"]
pub(crate) mod test_remote;
#[cfg(test)]
#[path = "os/shared/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "os/shared/sync_reliability_task_backend_fixture.rs"]
mod sync_reliability_task_backend_fixture;
#[cfg(test)]
#[path = "os/shared/sync_reliability_task_fixture.rs"]
mod sync_reliability_task_fixture;
#[cfg(test)]
#[path = "os/shared/sync_reliability_task_options_tests.rs"]
mod sync_reliability_task_options_tests;
#[cfg(test)]
#[path = "os/shared/sync_reliability_task_protection_tests.rs"]
mod sync_reliability_task_protection_tests;
#[cfg(test)]
#[path = "os/shared/sync_reliability_task_resume_tests.rs"]
mod sync_reliability_task_resume_tests;
