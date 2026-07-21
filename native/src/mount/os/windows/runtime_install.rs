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

    if exit_code == 0 {
        DokanyRuntime::preflight().map_err(|error| {
            format!(
                "Dokany-Installer war erfolgreich, aber API 231 ist danach nicht einsatzbereit: {error}"
            )
        })?;
    }
    Ok(DriveRuntimeInstallOutcome::from_msi_exit_code(exit_code))
}
