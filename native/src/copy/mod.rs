#[path = "os/shared/copy.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "os/linux_os.rs"]
mod platform;

#[path = "os/shared/publication_proof.rs"]
mod publication_proof;
pub(crate) use publication_proof::{
    confirm_acknowledged_transfer_publication, confirm_transfer_publication,
    confirm_transfer_publication_handle, confirm_transfer_publication_identity,
};
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;

pub use imp::*;

/// What a caller already learned of a copy's source from its open handle
/// (M10, 2026-10-28): the platform copy then neither probes the medium nor
/// reads the metadata again.
///
/// Only the Windows copy by path (`CopyFile2`) consumes the facts; the Linux
/// copy always fills the stage through handles (`copy_by_path` is `None`
/// there), so the fields are read on Windows alone.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct SourceFacts {
    pub size: u64,
    pub optical: bool,
}
// The streaming transfer engine runs local files through these.
pub(crate) use imp::{
    copy_to_new_file, discard_transfer_stage, local_file_identity, local_file_identity_handle,
    move_folder, native_path_text, packet_start, path_names_handle, prune_empty_dirs,
    recover_transfer_quarantine, stage_local_from, target_still_decided, transfer_local,
    transfer_local_fresh, transfer_local_from, transfer_local_observed, transfer_local_replacing,
    validate_directory_target, LocalFailure, LocalOutcome, LocalRequest, PacketStage,
    PacketStaging, PublicationObserver,
};
