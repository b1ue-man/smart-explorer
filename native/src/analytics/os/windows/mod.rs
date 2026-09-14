mod directory;
mod directory_records;
mod elevation;
mod image_lock;
mod paths;
mod privilege;

pub(super) use directory::read_directory;
pub(super) use paths::{display_path, normalize_scan_root};
pub(super) use elevation::{
    can_request_elevation, launch_elevated_analysis, verify_analysis_startup,
};
pub(super) use privilege::parallel_scan_allowed;

#[cfg(test)]
mod access_task;
