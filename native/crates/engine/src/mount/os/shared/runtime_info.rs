//! Shared filesystem-runtime queries delegating to the existing OS adapters.
#[cfg(any(windows, target_os = "linux"))]
use super::os;
use super::{DriveRuntimeInfo, DriveRuntimeInstallOutcome};

/// Windows mounts drives through Dokany, Linux through FUSE (DRV-M4);
/// Android offers folders through its documents provider instead (DRV-M8).
pub const fn drive_mount_supported() -> bool {
    cfg!(any(windows, target_os = "linux"))
}

/// The name of the device-tree drive (DRV-M6): Windows shows it as a volume
/// label (at most 32 characters), Linux as the folder `~/SmartExplorer/Geräte`.
pub fn devices_label() -> &'static str {
    if cfg!(windows) {
        "Smart Explorer Geräte"
    } else {
        "Geräte"
    }
}

/// Whether the filesystem runtime is installed and usable now (Dokany on
/// Windows, FUSE on Linux); the device tree is switched on by default only
/// then. Nothing is installed or asked for here.
pub fn drive_runtime_ready() -> bool {
    #[cfg(windows)]
    return drive_runtime_info().is_ok();
    #[cfg(target_os = "linux")]
    return os::linux_os::fuse_runtime_report(false).ready;
    #[cfg(not(any(windows, target_os = "linux")))]
    false
}

/// What the window shows about the filesystem runtime; `None` when it is
/// ready. While se installs it (install-on-use, Dokany 2.3.1 on Windows,
/// fuse3 on Linux) the progress, after a declined or failed install its
/// reason, otherwise that se installs it at the next mount. Nothing is
/// installed or asked for here.
pub fn drive_runtime_note() -> Option<String> {
    #[cfg(windows)]
    return os::windows::runtime_note();
    #[cfg(target_os = "linux")]
    return os::linux_os::fuse_runtime_note();
    #[cfg(not(any(windows, target_os = "linux")))]
    None
}

/// Whether se installs the filesystem runtime now (install-on-use): a caller
/// that waits for a mount waits for the install too.
pub fn drive_runtime_installing() -> bool {
    #[cfg(windows)]
    return os::windows::runtime_installing();
    #[cfg(target_os = "linux")]
    return os::linux_os::fuse_installing();
    #[cfg(not(any(windows, target_os = "linux")))]
    false
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
