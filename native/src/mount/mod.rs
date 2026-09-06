//! Remote-drive mounting boundaries.

#[path = "core/cache_policy.rs"]
mod cache_policy;
#[path = "core/cache_space.rs"]
mod cache_space;
#[path = "core/clean_cache.rs"]
mod clean_cache;
#[path = "core/entry_lifecycle.rs"]
mod entry_lifecycle;
#[path = "core/engine_recovery.rs"]
mod engine_recovery;
#[path = "core/file_commit.rs"]
mod file_commit;
#[path = "core/materialization.rs"]
mod materialization;
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
#[path = "core/metadata_cache.rs"]
mod metadata_cache;
#[path = "core/metadata_batch.rs"]
mod metadata_batch;
#[cfg(test)]
#[path = "core/vault_scheduler_task_tests.rs"]
mod vault_scheduler_task_tests;
#[path = "core/metadata_loading.rs"]
mod metadata_loading;
#[path = "core/metadata_point_cache.rs"]
mod metadata_point_cache;
#[path = "core/metadata_policy.rs"]
mod metadata_policy;
#[path = "core/mutations.rs"]
mod mutations;
#[path = "core/namespace_recovery.rs"]
mod namespace_recovery;
#[path = "core/open_handle.rs"]
mod open_handle;
#[path = "core/path.rs"]
mod path;
#[path = "core/recovery.rs"]
mod recovery;
#[path = "core/recovery_state.rs"]
mod recovery_state;
#[path = "core/replace.rs"]
mod replace;
#[path = "core/spool.rs"]
mod spool;
#[path = "core/startup.rs"]
mod startup;
#[path = "core/types.rs"]
mod types;
#[path = "core/windows_access.rs"]
mod windows_access;
#[path = "core/windows_case.rs"]
mod windows_case;

#[cfg(all(test, not(windows)))]
#[path = "core/metadata_cache_task_tests.rs"]
mod metadata_cache_task_tests;

#[cfg(all(test, not(windows)))]
#[path = "core/metadata_snapshot_task_tests.rs"]
mod metadata_snapshot_task_tests;

#[cfg(all(test, not(windows)))]
#[path = "core/navigation_cache_task_tests.rs"]
mod navigation_cache_task_tests;

#[cfg(all(test, not(windows)))]
#[path = "core/remote_drive_task_tests.rs"]
mod remote_drive_task_tests;

pub(crate) mod os;

#[cfg(test)]
#[path = "core/optimization_fixture.rs"]
pub(crate) mod optimization_fixture;
#[cfg(test)]
#[path = "core/optimization_cache_tests.rs"]
mod optimization_cache_tests;
#[cfg(test)]
#[path = "core/optimization_policy_tests.rs"]
mod optimization_policy_tests;
#[cfg(test)]
#[path = "core/optimization_metadata_tests.rs"]
mod optimization_metadata_tests;

pub use engine::MountEngine;
pub use cache_policy::{
    MountCachePolicy, MountRuntimePreference, DEFAULT_MOUNT_CACHE_MIB, MAX_MOUNT_CACHE_MIB,
};
pub use cache_space::CacheSpaceProbe;
pub use metadata_policy::{
    MountMetadataPolicy, DEFAULT_METADATA_PRELOAD_DEPTH, MAX_METADATA_PRELOAD_DEPTH,
};
pub use path::{PathProjector, ProjectedPath};
pub use recovery_state::{MountRecovery, MountSnapshot};
pub use spool::prepare_spool_root;
pub use types::{
    BackendRoot, Baseline, DeleteToken, DriveLetter, DriveSelection, EntryCondition, FlushOutcome,
    HandleId, MountConfig, MountConflict, MountId, MountMode, MountRootSecurity,
    MountRuntimeConfig, MountSource, MountStatus, NamespaceOutcome, OpenDisposition,
    OpenFileOptions, PeerMountTarget, RenameOutcome,
};
#[cfg_attr(not(windows), allow(unused_imports))]
pub(crate) use windows_access::{
    maximum_allowed_full_grant, maximum_allowed_read_grant, requests_maximum_allowed,
};
pub(crate) use windows_case::{validate_windows_case_component, windows_ordinal_key};

pub(crate) const DOKANY_LIBRARY_API_VERSION: u32 = 231;
pub(crate) const DOKANY_DRIVER_PROTOCOL_VERSION: u32 = 0x0000_0190;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DokanyVersionCompatibilityError {
    LibraryApiMismatch { expected: u32, found: u32 },
    DriverUnavailable,
    DriverProtocolMismatch { expected: u32, found: u32 },
}

/// Validates the two independent Dokany version domains without loading any
/// Windows components, so compatibility policy can be tested cross-platform.
pub(crate) const fn validate_dokany_version_domains(
    library_api: u32,
    driver_protocol: u32,
) -> Result<(), DokanyVersionCompatibilityError> {
    if library_api != DOKANY_LIBRARY_API_VERSION {
        return Err(DokanyVersionCompatibilityError::LibraryApiMismatch {
            expected: DOKANY_LIBRARY_API_VERSION,
            found: library_api,
        });
    }
    if driver_protocol == 0 {
        return Err(DokanyVersionCompatibilityError::DriverUnavailable);
    }
    if driver_protocol != DOKANY_DRIVER_PROTOCOL_VERSION {
        return Err(DokanyVersionCompatibilityError::DriverProtocolMismatch {
            expected: DOKANY_DRIVER_PROTOCOL_VERSION,
            found: driver_protocol,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct DriveRuntimeInfo {
    /// Legacy machine-output field retained for patch-release compatibility.
    pub required_api: u32,
    pub required_library_api: u32,
    pub library_api: u32,
    /// Legacy name retained for compatibility; this value is the driver
    /// protocol revision returned by `DokanDriverVersion()`.
    pub driver_api: u32,
    pub required_driver_protocol: u32,
    pub driver_protocol: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DriveRuntimeInstallOutcome {
    AlreadyReady,
    Installed { code: u32 },
    RebootRequired { code: u32 },
    RestartInitiated { code: u32 },
    Cancelled { code: u32 },
    AnotherInstallationRunning { code: u32 },
    FatalInstallerError { code: u32 },
    UnsupportedPlatform { code: u32 },
    UnexpectedFailure { code: u32 },
}

impl DriveRuntimeInstallOutcome {
    pub(crate) const fn from_msi_exit_code(code: u32) -> Self {
        match code {
            0 => Self::Installed { code: 0 },
            3010 => Self::RebootRequired { code: 3010 },
            1641 => Self::RestartInitiated { code: 1641 },
            1223 | 1602 => Self::Cancelled { code },
            1618 => Self::AnotherInstallationRunning { code: 1618 },
            1603 => Self::FatalInstallerError { code: 1603 },
            1633 | 1654 => Self::UnsupportedPlatform { code },
            code => Self::UnexpectedFailure { code },
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::AlreadyReady | Self::Installed { .. } => 0,
            Self::RebootRequired { code }
            | Self::RestartInitiated { code }
            | Self::Cancelled { code }
            | Self::AnotherInstallationRunning { code }
            | Self::FatalInstallerError { code }
            | Self::UnsupportedPlatform { code } => *code as i32,
            Self::UnexpectedFailure { .. } => 1,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::AlreadyReady => format!(
                "Dokany 2.3.1 (DLL-API {DOKANY_LIBRARY_API_VERSION}, Treiberprotokoll {DOKANY_DRIVER_PROTOCOL_VERSION}) ist bereits einsatzbereit."
            ),
            Self::Installed { .. } => format!(
                "Dokany 2.3.1 (DLL-API {DOKANY_LIBRARY_API_VERSION}, Treiberprotokoll {DOKANY_DRIVER_PROTOCOL_VERSION}) wurde installiert und ist einsatzbereit."
            ),
            Self::RebootRequired { .. } => {
                "Dokany wurde installiert. Windows muss neu gestartet werden.".into()
            }
            Self::RestartInitiated { .. } => {
                "Dokany wurde installiert; Windows hat den Neustart eingeleitet.".into()
            }
            Self::Cancelled { .. } => "Dokany-Installation wurde abgebrochen.".into(),
            Self::AnotherInstallationRunning { .. } => {
                "Eine andere Windows-Installation laeuft bereits (MSI 1618).".into()
            }
            Self::FatalInstallerError { .. } => {
                "Dokany-Installation ist mit einem schwerwiegenden MSI-Fehler fehlgeschlagen (1603)."
                    .into()
            }
            Self::UnsupportedPlatform { code } => format!(
                "Diese Dokany-Laufzeit wird von diesem Windows-System nicht unterstuetzt (MSI {code})."
            ),
            Self::UnexpectedFailure { code } => {
                format!("Dokany-Installation ist mit MSI-Code {code} fehlgeschlagen.")
            }
        }
    }

    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            Self::AnotherInstallationRunning { .. }
                | Self::FatalInstallerError { .. }
                | Self::UnsupportedPlatform { .. }
                | Self::UnexpectedFailure { .. }
        )
    }
}

pub const fn drive_mount_supported() -> bool {
    cfg!(windows)
}

pub fn drive_runtime_info() -> Result<DriveRuntimeInfo, String> {
    #[cfg(windows)]
    {
        let info = os::windows::preflight_runtime().map_err(|error| error.to_string())?;
        return Ok(DriveRuntimeInfo {
            required_api: info.required_library_api,
            required_library_api: info.required_library_api,
            library_api: info.library_api,
            driver_api: info.driver_protocol,
            required_driver_protocol: info.required_driver_protocol,
            driver_protocol: info.driver_protocol,
        });
    }
    #[cfg(not(windows))]
    Err("virtual drive hosts are supported only on Windows".to_string())
}

pub fn install_drive_runtime(
    local_msi: Option<&std::path::Path>,
) -> Result<DriveRuntimeInstallOutcome, String> {
    #[cfg(windows)]
    return os::windows::install_runtime(local_msi);
    #[cfg(not(windows))]
    {
        let _ = local_msi;
        Err("Dokany kann nur unter Windows installiert werden".to_string())
    }
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
