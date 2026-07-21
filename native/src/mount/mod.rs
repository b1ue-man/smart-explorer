//! Remote-drive mounting boundaries.

#[path = "core/case_semantics.rs"]
mod case_semantics;
#[path = "core/commit.rs"]
mod commit;
#[path = "core/delete.rs"]
mod delete;
#[path = "core/delete_recovery.rs"]
mod delete_recovery;
#[path = "core/engine.rs"]
mod engine;
#[path = "core/file_io.rs"]
mod file_io;
#[path = "core/journal.rs"]
mod journal;
#[path = "core/metadata.rs"]
mod metadata;
#[path = "core/mutations.rs"]
mod mutations;
#[path = "core/namespace_recovery.rs"]
mod namespace_recovery;
#[path = "core/path.rs"]
mod path;
#[path = "core/recovery.rs"]
mod recovery;
#[path = "core/replace.rs"]
mod replace;
#[path = "core/spool.rs"]
mod spool;
#[path = "core/types.rs"]
mod types;
#[path = "core/windows_case.rs"]
mod windows_case;

#[cfg(all(test, not(windows)))]
#[path = "core/remote_drive_task_tests.rs"]
mod remote_drive_task_tests;

pub(crate) mod os;

pub use engine::MountEngine;
pub use path::{PathProjector, ProjectedPath};
pub use spool::prepare_spool_root;
pub use types::{
    BackendRoot, Baseline, DeleteToken, DriveLetter, DriveSelection, EntryCondition, FlushOutcome,
    HandleId, MountConfig, MountConflict, MountId, MountMode, MountRootSecurity,
    MountRuntimeConfig, MountSnapshot, MountSource, MountStatus, NamespaceOutcome, OpenDisposition,
    OpenFileOptions, PeerMountTarget, RenameOutcome,
};
pub(crate) use windows_case::{validate_windows_case_component, windows_ordinal_key};

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct DriveRuntimeInfo {
    pub required_api: u32,
    pub library_api: u32,
    pub driver_api: u32,
}

pub const fn drive_mount_supported() -> bool {
    cfg!(windows)
}

pub fn drive_runtime_info() -> Result<DriveRuntimeInfo, String> {
    #[cfg(windows)]
    {
        let info = os::windows::preflight_runtime().map_err(|error| error.to_string())?;
        return Ok(DriveRuntimeInfo {
            required_api: os::windows::DOKANY_API_VERSION,
            library_api: info.library_api,
            driver_api: info.driver_api,
        });
    }
    #[cfg(not(windows))]
    Err("virtual drive hosts are supported only on Windows".to_string())
}

/// Recognizes the exact private invocation used by the daemon to start the
/// isolated Windows filesystem host. Authentication still requires the
/// one-use capability environment created by the daemon.
pub fn run_host_if_requested(arguments: &[std::ffi::OsString]) -> Option<Result<(), String>> {
    if arguments.len() != 2 || arguments[0] != std::ffi::OsStr::new("--mount-host") {
        return None;
    }
    let id = arguments[1]
        .to_str()
        .ok_or_else(|| "mount host id is not valid Unicode".to_string())
        .and_then(|value| MountId::parse(value).map_err(|error| error.to_string()));
    #[cfg(windows)]
    return Some(id.and_then(os::windows::run_mount_host));
    #[cfg(not(windows))]
    Some(id.and_then(|_| Err("virtual drive hosts are supported only on Windows".to_string())))
}
