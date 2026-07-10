#[path = "os/shared/apply.rs"]
mod apply;
#[path = "os/shared/archive.rs"]
mod archive;
#[path = "os/shared/config.rs"]
mod config;
#[path = "core/core.rs"]
mod core;
#[path = "os/shared/feed.rs"]
mod feed;
#[path = "os/shared/flow.rs"]
mod flow;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod os;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod os;
#[path = "os/shared/staging.rs"]
mod staging;
#[path = "os/shared/startup_ack.rs"]
mod startup_ack;
#[path = "core/types.rs"]
mod types;

pub use apply::apply_staged_update;
#[allow(unused_imports)]
pub use archive::{
    archive_current_version, cleanup_old_binaries, is_auto_update_paused, list_archived_versions,
    pinned_version, resume_auto_update,
};
pub use config::{set_update_source, take_updater_error, update_source_str};
pub use core::is_newer;
pub use feed::{download_update, download_version, list_remote_versions};
pub use flow::{check_async, update_to_latest_async};
pub use os::revert_to;
pub use staging::{discard_staged_update, load_staged_update, verify_staged_update};
pub(crate) use startup_ack::{
    acknowledge_update_startup, capture_update_startup_ack, update_startup_ack_pending,
};
pub use types::{StagedUpdate, UpdateMsg, VerifiedPayload};

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
