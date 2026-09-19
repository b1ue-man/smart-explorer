mod broker;
mod directory;
mod directory_records;
mod elevation;
mod image_lock;
mod paths;
mod pipe;
mod privilege;
mod read;

pub(crate) use directory::read_directory;
pub(crate) use elevation::{can_request_access, request_access, run_helper_if_requested};
pub(crate) use paths::{display_path, normalize_scan_root};
pub(crate) use privilege::parallel_scan_allowed;
pub(crate) use read::{open_read, symlink_metadata};

#[cfg(test)]
mod access_task;
#[cfg(test)]
mod helper_task;
