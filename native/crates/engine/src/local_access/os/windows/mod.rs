mod broker;
mod confined;
pub use confined::ConfinedDirectory;
mod directory;
mod directory_handle;
mod directory_records;
mod directory_rename;
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

#[cfg(test)]
mod access_task;
#[cfg(test)]
mod helper_task;
