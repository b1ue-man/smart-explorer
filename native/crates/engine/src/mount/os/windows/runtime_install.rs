//! The pinned Dokany 2.3.1 MSI behind its UAC prompt: `se drive
//! install-runtime`, the window's button and install-on-use (user order
//! 2026-11-24): a drive, the device tree or a vault drive whose host finds
//! no Dokany installs it at that moment and loads the runtime once more. One
//! install at a time for this account (`login/os/shared/install_gate.rs`);
//! parallel drives wait for it and take its result. A different Dokany
//! runtime that is already installed is never replaced.
use std::path::{Path, PathBuf};

use crate::login::os::shared::install_gate::{with_install, Attempt, Failure, Gate, Progress};
use crate::mount::{
    DriveRuntimeInstallOutcome, MountId, MountRuntimePreference, DOKANY_DRIVER_PROTOCOL_VERSION,
    DOKANY_LIBRARY_API_VERSION,
};

use super::{
    runtime::{DokanyPreflightError, DokanyRuntime},
    runtime_install_download::{acquire_msi, pinned_msi},
    runtime_install_process::{ElevatedLaunchError, LockedMsi},
    runtime_selection::RuntimeSelection,
};

const GATE: &str = "dokany";

/// Where an install keeps its gate and its download.
struct Place {
    gate: PathBuf,
    download: PathBuf,
}

impl Place {
    /// A process with the account's environment (desktop, `se`, worker).
    fn account() -> Self {
        Self {
            gate: crate::support_dirs::app_data_dir(),
            download: std::env::temp_dir()
                .join("smart-explorer")
                .join("runtime-install"),
        }
    }

    /// The drive host runs with a cleared environment (only `SystemRoot`,
    /// `WINDIR` and its own values, `daemon/os/windows/
    /// mount_process_environment.rs`): neither `APPDATA` nor `TEMP` exist
    /// there. Its cache root is `<app data>\mount-cache`
    /// (`daemon/os/shared/mount_manager_start_cache.rs`), so the app data
    /// folder, and with it the same gate as every other process, is its
    /// parent; the download goes below it.
    fn host(cache_root: &Path) -> Result<Self, String> {
        let parent = cache_root
            .parent()
            .ok_or_else(|| "Laufwerk-Cache hat keinen App-Datenordner".to_string())?;
        let app_data = crate::transfer::canonical_folder(parent)
            .map_err(|error| format!("App-Datenordner bestimmen: {error}"))?;
        Ok(Self {
            download: app_data.join("runtime-install"),
            gate: app_data,
        })
    }
}

/// Only an absent runtime or driver is installed; any other preflight
/// error is a different runtime se never replaces.
fn absent(error: &DokanyPreflightError) -> bool {
    matches!(
        error,
        DokanyPreflightError::RuntimeNotInstalled | DokanyPreflightError::DriverUnavailable
    )
}

pub fn install_runtime(local_msi: Option<&Path>) -> Result<DriveRuntimeInstallOutcome, String> {
    install_at(&Place::account(), local_msi)
}

fn install_at(
    place: &Place,
    local_msi: Option<&Path>,
) -> Result<DriveRuntimeInstallOutcome, String> {
    let mut direct = None;
    let gate = Gate::at(place.gate.clone(), GATE);
    let attempt = gate.run(&|| DokanyRuntime::preflight().is_ok(), || {
        let result = install_now(local_msi, &place.download);
        let attempt = attempt_of(&result);
        direct = Some(result);
        attempt
    });
    // Present already, or the result of an install this call waited for.
    match (direct, attempt) {
        (Some(result), _) => result,
        (None, Attempt::Installed) => Ok(DriveRuntimeInstallOutcome::AlreadyReady),
        (None, Attempt::Declined(_)) => Ok(DriveRuntimeInstallOutcome::Cancelled { code: 1223 }),
        (None, Attempt::Failed(reason)) => Err(reason),
    }
}

fn attempt_of(result: &Result<DriveRuntimeInstallOutcome, String>) -> Attempt {
    match result {
        Ok(DriveRuntimeInstallOutcome::AlreadyReady | DriveRuntimeInstallOutcome::Installed { .. }) => {
            Attempt::Installed
        }
        Ok(DriveRuntimeInstallOutcome::Cancelled { .. }) => Attempt::Declined(
            "Die Installation von Dokany 2.3.1 wurde in der Windows-Administratorabfrage abgelehnt."
                .into(),
        ),
        Ok(other) => Attempt::Failed(other.message()),
        Err(reason) => Attempt::Failed(reason.clone()),
    }
}

fn install_now(
    local_msi: Option<&Path>,
    download: &Path,
) -> Result<DriveRuntimeInstallOutcome, String> {
    match DokanyRuntime::preflight() {
        Ok(_) => return Ok(DriveRuntimeInstallOutcome::AlreadyReady),
        Err(error) if absent(&error) => {}
        Err(error) => {
            return Err(format!(
                "vorhandene Dokany-Laufzeit wird nicht automatisch ersetzt: {error}"
            ))
        }
    }

    let pinned = pinned_msi()?;
    let artifact = acquire_msi(local_msi, &pinned, download)?;
    let locked = LockedMsi::open(artifact.path(), &pinned)?;
    locked.verify_authenticode()?;
    let exit_code = match locked.run_elevated() {
        Ok(code) => code,
        Err(ElevatedLaunchError::Cancelled) => {
            return Ok(DriveRuntimeInstallOutcome::Cancelled { code: 1223 })
        }
        Err(ElevatedLaunchError::Other(error)) => return Err(error),
    };

    if exit_code == 0 {
        DokanyRuntime::preflight().map_err(|error| {
            format!(
                "Dokany-Installer war erfolgreich, aber DLL-API {DOKANY_LIBRARY_API_VERSION} / Treiberprotokoll {DOKANY_DRIVER_PROTOCOL_VERSION} ist danach nicht einsatzbereit: {error}"
            )
        })?;
    }
    Ok(DriveRuntimeInstallOutcome::from_msi_exit_code(exit_code))
}

/// The drive host's runtime: loaded, else Dokany installed (its UAC prompt)
/// and loaded once more. The error is the reason the drive shows. An absent
/// System32 runtime is found before the selection: the selection arms the
/// private runtime's attempt marker, which a missing driver would leave
/// behind and so turn this drive's later mounts to the non-batched runtime.
/// Any other preflight result leaves the choice to the selection as before.
pub(super) fn select_installing(
    cache_root: &Path,
    id: &MountId,
    preference: MountRuntimePreference,
) -> Result<RuntimeSelection, String> {
    let classify = |error: DokanyPreflightError| {
        if absent(&error) {
            Failure::Missing(error.to_string())
        } else {
            Failure::Other(error.to_string())
        }
    };
    with_install(
        || {
            if let Err(error) = DokanyRuntime::preflight() {
                if absent(&error) {
                    return Err(classify(error));
                }
            }
            RuntimeSelection::select(cache_root, id, preference).map_err(classify)
        },
        || {
            // A test build never downloads or installs a system driver.
            if cfg!(any(test, feature = "test-support")) {
                return Attempt::Failed(
                    "Dokany 2.3.1 wird in einem Testbau nicht installiert.".into(),
                );
            }
            match Place::host(cache_root) {
                Ok(place) => attempt_of(&install_at(&place, None)),
                Err(reason) => Attempt::Failed(reason),
            }
        },
    )
}

/// Whether an install of Dokany runs now (any process of this account).
pub fn runtime_installing() -> bool {
    Gate::named(GATE).progress() == Progress::Running
}

/// The window's note about Dokany; `None` when it is ready.
pub fn runtime_note() -> Option<String> {
    match DokanyRuntime::preflight() {
        Ok(_) => None,
        Err(error) if absent(&error) => Some(match Gate::named(GATE).progress() {
            Progress::Running => "Dokany 2.3.1 wird installiert …".to_string(),
            Progress::Ended(Attempt::Declined(reason) | Attempt::Failed(reason)) => reason,
            Progress::Idle | Progress::Ended(Attempt::Installed) => {
                "Dokany 2.3.1 fehlt; se installiert es beim nächsten Einbinden – Windows fragt dann nach der Administratorfreigabe."
                    .to_string()
            }
        }),
        Err(error) => Some(format!(
            "Dokany ist nicht nutzbar: {error}. Eine vorhandene andere Dokany-Laufzeit ersetzt se nicht."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{absent, attempt_of, Attempt, DokanyPreflightError, DriveRuntimeInstallOutcome};

    /// Only an absent runtime or driver is installed on use; the MSI's
    /// answers become the reason the drive shows (a declined UAC prompt is a
    /// decline, a restart or MSI failure a failure with its message).
    #[test]
    fn dokany_install_task_outcomes_and_never_replace() {
        assert!(absent(&DokanyPreflightError::RuntimeNotInstalled));
        assert!(absent(&DokanyPreflightError::DriverUnavailable));
        for different in [
            DokanyPreflightError::LibraryApiMismatch {
                expected: 231,
                found: 230,
            },
            DokanyPreflightError::DriverProtocolMismatch {
                expected: 0x190,
                found: 0x180,
            },
            DokanyPreflightError::RuntimeArchitectureMismatch,
        ] {
            assert!(!absent(&different), "{different:?}");
        }
        assert_eq!(
            attempt_of(&Ok(DriveRuntimeInstallOutcome::Installed { code: 0 })),
            Attempt::Installed
        );
        assert_eq!(
            attempt_of(&Ok(DriveRuntimeInstallOutcome::Cancelled { code: 1223 })),
            Attempt::Declined(
                "Die Installation von Dokany 2.3.1 wurde in der Windows-Administratorabfrage abgelehnt."
                    .into()
            )
        );
        assert_eq!(
            attempt_of(&Ok(DriveRuntimeInstallOutcome::RebootRequired {
                code: 3010
            })),
            Attempt::Failed("Dokany wurde installiert. Windows muss neu gestartet werden.".into())
        );
        assert_eq!(
            attempt_of(&Err("Dokany 2.3.1.1000 herunterladen: offline".into())),
            Attempt::Failed("Dokany 2.3.1.1000 herunterladen: offline".into())
        );
    }
}
