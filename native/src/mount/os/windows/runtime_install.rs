use std::path::Path;

use crate::mount::DriveRuntimeInstallOutcome;

use super::{
    runtime::{DokanyPreflightError, DokanyRuntime},
    runtime_install_download::{acquire_msi, pinned_msi},
    runtime_install_process::{ElevatedLaunchError, LockedMsi},
};

pub(crate) fn install_runtime(
    local_msi: Option<&Path>,
) -> Result<DriveRuntimeInstallOutcome, String> {
    match DokanyRuntime::preflight() {
        Ok(_) => return Ok(DriveRuntimeInstallOutcome::AlreadyReady),
        Err(
            DokanyPreflightError::RuntimeNotInstalled | DokanyPreflightError::DriverUnavailable,
        ) => {}
        Err(error) => {
            return Err(format!(
                "vorhandene Dokany-Laufzeit wird nicht automatisch ersetzt: {error}"
            ))
        }
    }

    let pinned = pinned_msi()?;
    let artifact = acquire_msi(local_msi, &pinned)?;
    let locked = LockedMsi::open(artifact.path(), &pinned)?;
    locked.verify_authenticode()?;
    let exit_code = match locked.run_elevated() {
        Ok(code) => code,
        Err(ElevatedLaunchError::Cancelled) => {
            return Ok(DriveRuntimeInstallOutcome::Cancelled { code: 1223 })
        }
        Err(ElevatedLaunchError::Other(error)) => return Err(error),
    };

    match exit_code {
        0 => {
            DokanyRuntime::preflight().map_err(|error| {
                format!(
                    "Dokany-Installer war erfolgreich, aber API 231 ist danach nicht einsatzbereit: {error}"
                )
            })?;
            Ok(DriveRuntimeInstallOutcome::Installed { code: 0 })
        }
        3010 => Ok(DriveRuntimeInstallOutcome::RebootRequired { code: 3010 }),
        1641 => Ok(DriveRuntimeInstallOutcome::RestartInitiated { code: 1641 }),
        1223 | 1602 => Ok(DriveRuntimeInstallOutcome::Cancelled { code: exit_code }),
        1618 => Ok(DriveRuntimeInstallOutcome::AnotherInstallationRunning { code: 1618 }),
        1603 => Ok(DriveRuntimeInstallOutcome::FatalInstallerError { code: 1603 }),
        1633 | 1654 => Ok(DriveRuntimeInstallOutcome::UnsupportedPlatform { code: exit_code }),
        code => Ok(DriveRuntimeInstallOutcome::UnexpectedFailure { code }),
    }
}
