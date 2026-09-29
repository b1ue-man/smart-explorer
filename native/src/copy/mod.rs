#[path = "os/shared/copy.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "os/linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;

pub use imp::*;
// The streaming transfer engine runs local files through these.
pub(crate) use imp::{
    copy_to_new_file, move_folder, prune_empty_dirs, transfer_local, validate_directory_target,
    LocalFailure, LocalOutcome, LocalRequest,
};
