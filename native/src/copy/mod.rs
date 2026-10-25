#[path = "os/shared/copy.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "os/linux_os.rs"]
mod platform;

#[path = "os/shared/publication_proof.rs"]
mod publication_proof;
pub(crate) use publication_proof::{confirm_transfer_publication, confirm_transfer_publication_handle};
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;

pub use imp::*;
// The streaming transfer engine runs local files through these.
pub(crate) use imp::{
    copy_to_new_file, discard_transfer_stage, local_file_identity, local_file_identity_handle,
    move_folder, native_path_text, prune_empty_dirs, recover_transfer_quarantine, transfer_local,
    transfer_local_observed, validate_directory_target,
    LocalFailure, LocalOutcome, LocalRequest, PublicationObserver,
};
