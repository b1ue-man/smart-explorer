mod broker;
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

pub(crate) use directory::classify_open_file;
pub(crate) use directory_handle::{secure_private_handle, DirectoryHandle, QuarantinedChild};

pub(crate) use directory::{metadata_class, metadata_is_link_like, read_directory};
pub(crate) use elevation::{can_request_access, request_access, run_helper_if_requested};
pub(crate) use paths::{display_path, normalize_scan_root};
pub(crate) use privilege::parallel_scan_allowed;
pub(crate) use read::{open_read, open_regular, symlink_metadata};

#[cfg(test)]
mod access_task;
#[cfg(test)]
mod helper_task;
