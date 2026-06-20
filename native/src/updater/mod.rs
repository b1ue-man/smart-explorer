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
#[path = "core/types.rs"]
mod types;

#[allow(unused_imports)]
pub use archive::{
    archive_current_version, cleanup_old_binaries, is_auto_update_paused, list_archived_versions,
    pinned_version, resume_auto_update,
};
pub use config::{set_update_source, take_updater_error, update_source_str};
pub use core::is_newer;
pub use feed::{download_version, list_remote_versions};
pub use flow::{check_async, update_to_latest_async};
pub use os::{install_version, revert_to, run_apply_worker};
pub use types::UpdateMsg;

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
