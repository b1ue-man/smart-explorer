//! Remote transfers without GUI state: the concurrent transfer lane, the
//! upload, download and remote-to-remote copy workers, private staging with
//! create-only commits, progress, cancellation, and the app-owned temp copies
//! transfers bridge through. The desktop GUI and the mobile facade drive the
//! same workers.
//!
//! Every walk leaves the active app trash out as a protected omission
//! (`apptrash::excluded_name`, counted in `TransferProgress::omitted`); the
//! desktop builds never activate the app trash.

#[path = "os/shared/cancel.rs"]
mod cancel;
#[path = "os/shared/copy_commit.rs"]
mod copy_commit;
#[path = "os/shared/download_file.rs"]
mod download_file;
#[path = "os/shared/downloads.rs"]
mod downloads;
#[path = "os/shared/entries.rs"]
mod entries;
#[path = "os/shared/lane.rs"]
mod lane;
#[path = "os/shared/local_stage.rs"]
mod local_stage;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "os/unix.rs"]
mod platform;
#[path = "os/shared/progress.rs"]
mod progress;
#[path = "os/shared/remote_copy.rs"]
mod remote_copy;
#[path = "os/shared/snapshot_download.rs"]
mod snapshot_download;
#[path = "os/shared/temp.rs"]
mod temp;
#[path = "os/shared/temp_delete.rs"]
mod temp_delete;
#[path = "core/types.rs"]
mod types;
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

#[cfg(test)]
#[path = "os/shared/cancel_tests.rs"]
mod cancel_tests;
#[cfg(test)]
#[path = "os/shared/copy_paste_task_backend.rs"]
mod copy_paste_task_backend;
#[cfg(test)]
#[path = "os/shared/copy_paste_task_tests.rs"]
mod copy_paste_task_tests;
#[cfg(test)]
#[path = "os/shared/tests.rs"]
mod tests;

pub use downloads::{
    download_paths_progress, download_remote_clipboard_items, download_remote_paths_for_clipboard,
};
pub use lane::{
    launch_transfer, ActiveTransfer, Admission, FinishedTransfer, LaunchTransfer, TransferLane,
    TransferRequest, MAX_ACTIVE_TRANSFERS,
};
pub use local_stage::download_to_id;
pub(crate) use platform::{replace_file_atomic, upload_is_link_like};
pub use remote_copy::copy_remote_paths_progress;
pub use snapshot_download::download_clipboard_snapshot;
pub use temp::{
    cleanup_session_temp, cleanup_temp_copy, open_temp_path, safe_temp_name, session_temp_dir,
    temp_root, TEMP_SESSION_PID_FILE,
};
pub(crate) use temp::{session_marker_path, session_tag, write_session_marker};
pub(crate) use temp_delete::{remove_owned_tree, remove_owned_tree_controlled};
pub use types::{TransferKind, TransferMsg, TransferProgress};
pub use upload_pairs::upload_pairs_progress;
pub use upload_reader::upload_reader_progress;
pub use uploads::{upload_file, upload_paths_progress};
