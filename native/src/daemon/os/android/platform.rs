//! Android adapter for the background worker, which runs as a thread of the
//! app process. The POSIX primitives (atomic control replacement, no-replace
//! restore, link checks, the `flock` singleton) are the Linux ones; Android
//! differs in where the singleton lock lives (app-private storage instead of
//! the XDG runtime directory), which shell runs job hooks, and where the
//! auto-pause conditions come from (the host's `HostState`).

use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

// The Linux selections replaced below (XDG lock directory, `sh` on PATH,
// condition stubs) stay unused on Android.
#[cfg(target_os = "android")]
#[allow(dead_code)]
#[path = "../linux_os/platform.rs"]
mod posix;
// Linux host tests compile this adapter beside the Linux platform module,
// which is the same POSIX source.
#[cfg(not(target_os = "android"))]
use super::platform as posix;

pub use posix::DriveInfo;
pub(crate) use posix::{
    atomic_replace, metadata_is_link_like, normalize_local_backend_path, restore_control_if_absent,
    DaemonInstanceGuard,
};

const LOCK_DIRECTORY: &str = "daemon-lock";
const SHELL: &str = "/system/bin/sh";
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;

/// Removable-device triggers have no Android source yet; like Linux, none fire.
pub(crate) fn removable_drives() -> Vec<DriveInfo> {
    Vec::new()
}

pub(crate) fn battery_saver_on() -> bool {
    super::host_state::host_state().power_save
}

pub(crate) fn on_metered_network() -> bool {
    super::host_state::host_state().metered
}

/// Job hooks run inside the app sandbox with the system shell.
pub(crate) fn run_shell_command(cmd: &str) -> io::Result<std::process::ExitStatus> {
    std::process::Command::new(SHELL).args(["-c", cmd]).status()
}

pub(crate) fn acquire_daemon_instance_guard(
    timeout: std::time::Duration,
) -> Option<DaemonInstanceGuard> {
    posix::acquire_daemon_instance_guard_in(timeout, daemon_lock_directory)
}

fn daemon_lock_directory() -> io::Result<PathBuf> {
    private_directory(&crate::support_dirs::sync_data_dir().join(LOCK_DIRECTORY))
}

/// Create (or tighten) an owner-only directory; the shared lock code rejects
/// anything but a user-owned mode-0700 directory and never follows links.
fn private_directory(directory: &Path) -> io::Result<PathBuf> {
    match DirBuilder::new()
        .mode(PRIVATE_DIRECTORY_MODE)
        .create(directory)
    {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(directory)?;
    if !metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon lock directory must be a real directory",
        ));
    }
    std::fs::set_permissions(
        directory,
        std::fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE),
    )?;
    Ok(directory.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{battery_saver_on, on_metered_network, private_directory, removable_drives};
    use crate::daemon::{set_host_state, HostState};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn android_task_platform_reflects_host_state_and_private_lock_directory() {
        set_host_state(HostState {
            power_save: true,
            metered: false,
        });
        assert!(battery_saver_on());
        assert!(!on_metered_network());
        set_host_state(HostState {
            power_save: false,
            metered: true,
        });
        assert!(!battery_saver_on());
        assert!(on_metered_network());
        set_host_state(HostState::default());
        assert!(!battery_saver_on() && !on_metered_network());
        assert!(removable_drives().is_empty());

        let base = tempfile::tempdir().unwrap();
        let directory = base.path().join("daemon-lock");
        assert_eq!(private_directory(&directory).unwrap(), directory);
        let mode = std::fs::metadata(&directory).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o700);

        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_directory(&directory).unwrap();
        let mode = std::fs::metadata(&directory).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o700);

        let file = base.path().join("not-a-directory");
        std::fs::write(&file, b"").unwrap();
        assert!(private_directory(&file).is_err());
    }
}
