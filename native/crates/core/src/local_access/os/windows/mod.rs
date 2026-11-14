mod broker;
#[doc(hidden)]
pub mod confined;
pub use confined::ConfinedDirectory;
mod directory;
#[doc(hidden)]
pub mod directory_handle;
mod directory_records;
mod directory_rename;
#[doc(hidden)]
pub mod directory_watch;
mod elevation;
mod image_lock;
mod paths;
mod pipe;
mod privilege;
mod read;
mod regular;

pub use directory::classify_open_file;
pub use directory_handle::{secure_private_handle, DirectoryHandle, QuarantinedChild};

pub use directory::{metadata_class, metadata_is_link_like, read_directory};
pub use elevation::{can_request_access, request_access, run_helper_if_requested};
pub use paths::{display_path, normalize_scan_root};
pub use privilege::parallel_scan_allowed;
pub use read::{open_read, open_regular, symlink_metadata};

// The access tests moved to analytics; their shared fixtures stay here, because
// helper_task (this module) names them.
#[cfg(any(test, feature = "test-support"))]
pub mod access_fixtures;
#[cfg(any(test, feature = "test-support"))]
pub use privilege::BackupRead;
#[cfg(any(test, feature = "test-support"))]
mod access_task {
    // Old path of the fixture types, used by helper_task.
    pub(super) use super::access_fixtures::{DeniedDirectory, Identity};
}
#[cfg(test)]
mod helper_task;
