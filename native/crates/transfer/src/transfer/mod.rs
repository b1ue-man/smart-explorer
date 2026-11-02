//! Remote transfers without GUI state: the concurrent transfer lane, the
//! upload, download and remote-to-remote copy workers, private staging with
//! create-only commits, progress, cancellation, and the app-owned temp copies
//! transfers bridge through. The desktop GUI and the mobile facade drive the
//! same workers.
//!
//! Every walk leaves the active app trash out as a protected omission
//! (`apptrash::excluded_name`, counted in `TransferProgress::omitted`); the
//! desktop builds never activate the app trash.

#[path = "os/shared/access.rs"]
pub mod access;
#[path = "os/shared/cancel.rs"]
mod cancel;
#[path = "os/shared/clipboard_parallel.rs"]
mod clipboard_parallel;
#[path = "os/shared/copy_commit.rs"]
mod copy_commit;
#[path = "os/shared/download_file.rs"]
mod download_file;
#[path = "os/shared/downloads.rs"]
mod downloads;
#[path = "os/shared/edit_download.rs"]
mod edit_download;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
#[path = "os/shared/edit_download_task_source.rs"]
pub mod edit_download_task_source;
#[path = "os/shared/engine/mod.rs"]
pub mod engine;
#[path = "core/engine_names.rs"]
pub mod engine_names;
#[path = "core/engine_policy.rs"]
pub mod engine_policy;
#[path = "os/shared/entries.rs"]
pub mod entries;
#[path = "os/shared/external.rs"]
mod external;
#[path = "os/shared/flow.rs"]
pub mod flow;
#[path = "core/flow_control.rs"]
mod flow_control;
#[path = "os/shared/flow_registry.rs"]
mod flow_registry;
#[path = "core/flow_window.rs"]
mod flow_window;
#[path = "core/folder_alias.rs"]
mod folder_alias;
#[path = "core/folder_conflict.rs"]
mod folder_conflict;
#[path = "os/shared/hints.rs"]
mod hints;
#[path = "core/job.rs"]
mod job;
#[path = "core/job_record.rs"]
mod job_record;
pub use job_record::{file_rule, JobRecord, StoredEndpoint, StoredFilter};
#[path = "os/shared/lane.rs"]
mod lane;
#[path = "core/limiter.rs"]
pub mod limiter;
#[path = "os/shared/local_stage.rs"]
mod local_stage;
#[path = "core/ports.rs"]
pub mod ports;
#[path = "os/shared/progress.rs"]
mod progress;
#[path = "os/shared/read_pipeline.rs"]
pub mod read_pipeline;
#[path = "os/shared/remote_copy.rs"]
mod remote_copy;
#[path = "core/segments.rs"]
pub mod segments;
#[path = "os/shared/selection.rs"]
mod selection;
#[path = "os/shared/snapshot_download.rs"]
mod snapshot_download;
#[path = "os/shared/temp.rs"]
mod temp;
#[path = "os/shared/temp_delete.rs"]
mod temp_delete;
#[path = "core/time_window.rs"]
pub mod time_window;
#[path = "core/types.rs"]
mod types;
#[path = "core/update_rule.rs"]
pub mod update_rule;
#[path = "os/shared/upload_pairs.rs"]
mod upload_pairs;
#[path = "os/shared/upload_plan.rs"]
mod upload_plan;
#[path = "os/shared/upload_reader.rs"]
mod upload_reader;
#[path = "os/shared/upload_stream.rs"]
mod upload_stream;
#[path = "os/shared/uploads.rs"]
mod uploads;
#[path = "os/shared/walk.rs"]
pub mod walk;
#[path = "os/shared/walk_listers.rs"]
pub mod walk_listers;

#[cfg(test)]
#[path = "os/shared/cancel_tests.rs"]
mod cancel_tests;
#[cfg(test)]
#[path = "os/shared/clipboard_parallel_tests.rs"]
mod clipboard_parallel_tests;
#[cfg(any(test, feature = "test-support"))]
#[path = "os/shared/copy_paste_task_backend.rs"]
mod copy_paste_task_backend;
#[cfg(test)]
#[path = "os/shared/copy_paste_task_tests.rs"]
mod copy_paste_task_tests;
#[cfg(test)]
#[path = "core/limiter_tests.rs"]
mod limiter_tests;
#[cfg(test)]
#[path = "core/segments_tests.rs"]
mod segments_tests;
#[cfg(test)]
#[path = "os/shared/tests.rs"]
mod tests;

pub use access::{AccessAnswer, AccessGate};
pub use downloads::{
    download_paths_progress, download_remote_clipboard_items, download_remote_paths_for_clipboard,
};
pub use edit_download::download_for_edit;
pub use engine::new_job_id;
pub use engine::record_io::{
    monitor_contexts, read_record, read_record_state, record_context, record_locations,
    write_record, RecordContext, RecordKind,
};
pub use engine::stored_identity::{address_of, remember as remember_identity};
pub use engine::{count_folder_conflict, folder_conflicts, FolderPreviewScan};
pub use engine::{reopen_job, unconfirmed_jobs, RecoveryCatalog};
pub use engine::{run_view, run_view_here, JobView, Side};
pub use engine_names::parent_path;
pub use external::{external_snapshots, register_external, ExternalSnapshot, ExternalTransfer};
pub use flow::{
    acquire_pair, classify_error, flow, flow_for, flow_with_identity, local_flow, Flow, FlowPermit,
    FlowSnapshot, PermitPair, ANONYMOUS_JOB,
};
pub use flow_control::OpOutcome;
pub use folder_conflict::{FolderConflict, FolderConflictPolicy, FolderConflictPreview};
pub use job::{path_within, Endpoint, JobItems, Layout, PairItem, RecoveryMode, TransferJob};
pub use lane::{
    launch_transfer, ActiveTransfer, FinishedTransfer, LaunchTransfer, TransferLane,
    TransferRequest,
};
pub use limiter::Limiter;
pub use local_stage::download_to_id;
pub use ports::{TransferVault, TransferWorker, FIELDS_WITHHELD};
// The host adapter, memory budget, sensors and compression policy live in
// `crate::host` (core layer, D13); these lines keep their transfer paths.
use crate::host::platform;
pub use crate::host::{compress_policy, sensor_parse, sensors};
pub use crate::host::{memory_budget, reserve_memory, try_reserve_memory, MemoryReservation};
pub use platform::available_memory;
pub use platform::{
    canonical_folder, lower_thread_priority, physical_memory, replace_file_atomic,
    upload_is_link_like, write_all_at,
};
pub use remote_copy::copy_remote_paths_progress;
pub use selection::{ListedEntry, SelectionListing, SelectionSource};
pub use snapshot_download::download_clipboard_snapshot;
pub use temp::{
    cleanup_session_temp, cleanup_temp_copy, open_temp_path, safe_temp_name, session_temp_dir,
    temp_root, TEMP_SESSION_PID_FILE,
};
pub use temp::{session_marker_path, session_tag, write_session_marker};
pub use temp_delete::{remove_owned_tree, remove_owned_tree_controlled};
pub use types::{ResolvedRoot, TransferIssue, TransferKind, TransferMsg, TransferProgress};
pub use update_rule::summary as update_summary;
pub use upload_pairs::upload_pairs_progress;
pub use upload_reader::upload_reader_progress;
pub use uploads::{upload_file, upload_paths_progress};
