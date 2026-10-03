#[path = "shell.rs"]
mod shell;
pub(crate) use shell::{shell_command, spawn_shell};

pub(crate) fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new().create(true).append(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "log must be a regular file"));
    }
    Ok(file)
}

pub(crate) fn requires_storage_access(_endpoint: &str) -> bool { false }

use std::borrow::Cow;

#[cfg(debug_assertions)]
// The Credential Manager adapter includes this debug-only parser as well; one
// source keeps both isolated resource names on the same validation contract.
#[allow(clippy::duplicate_mod)]
#[path = "../../../windows_test_namespace.rs"]
mod test_namespace;

const DAEMON_MUTEX_NAME: &str = r"Local\SmartExplorerSyncDaemon";
#[cfg(debug_assertions)]
const TEST_MUTEX_SEPARATOR: &str = ".Test.";

#[derive(Clone)]
pub struct DriveInfo {
    pub letter: String,
    pub label: String,
    pub serial: String,
}

pub(crate) struct DaemonInstanceGuard(windows_sys::Win32::Foundation::HANDLE);

impl Drop for DaemonInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub(crate) fn removable_drives() -> Vec<DriveInfo> {
    drive_snapshot().unwrap_or_default()
}

pub(crate) fn battery_saver_on() -> bool {
    power::battery_saver_on()
}

pub(crate) fn on_metered_network() -> bool {
    power::on_metered_network()
}

/// (battery saver, metered network): both are read from the system.
pub(crate) fn autopause_conditions_supported() -> (bool, bool) {
    (true, true)
}

pub(crate) fn run_shell_command(cmd: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("cmd").args(["/C", cmd]).status()
}

/// The Windows IPC listener keeps its 100 ms poll; `timeout` is not used.
pub(crate) fn wait_for_ipc_client(
    _listener: &std::net::TcpListener,
    _timeout: std::time::Duration,
) -> std::io::Result<()> {
    std::thread::sleep(std::time::Duration::from_millis(100));
    Ok(())
}

/// The 100 ms poll sees the stop flag by itself; no wake connection needed.
pub(crate) fn wake_ipc_listener(_addr: std::net::SocketAddr) {}

/// Convert a host-native local path into the forward-slash form required by
/// the VFS boundary. Windows does not allow backslashes as filename characters,
/// so this preserves path identity for drive, UNC, and relative local paths.
pub(crate) fn normalize_local_backend_path(path: &str) -> Cow<'_, str> {
    if path.contains('\\') {
        Cow::Owned(path.replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    }
}

pub(crate) fn atomic_replace(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(crate) fn restore_control_if_absent(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok != 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        Ok(false)
    } else {
        Err(error)
    }
}

pub(crate) fn metadata_is_link_like(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

pub(crate) fn acquire_daemon_instance_guard(
    timeout: std::time::Duration,
) -> Option<DaemonInstanceGuard> {
    match try_acquire_daemon_mutex(timeout) {
        Ok(guard) => guard,
        Err(error) => {
            super::state::log(&format!("daemon single-instance lock failed: {error}"));
            None
        }
    }
}

fn try_acquire_daemon_mutex(
    timeout: std::time::Duration,
) -> std::io::Result<Option<DaemonInstanceGuard>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, WAIT_ABANDONED, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};

    let mutex_name = daemon_mutex_name()?;
    let name: Vec<u16> = std::ffi::OsStr::new(mutex_name.as_ref())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        // Opening an existing named mutex is not ownership. Wait on the mutex
        // so a replacement can perform a bounded handoff after the old daemon
        // closes IPC but before its process releases the singleton.
        let handle = CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr());
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let milliseconds = timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32;
        match WaitForSingleObject(handle, milliseconds) {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Some(DaemonInstanceGuard(handle))),
            WAIT_TIMEOUT => {
                CloseHandle(handle);
                Ok(None)
            }
            WAIT_FAILED => {
                let error = std::io::Error::last_os_error();
                CloseHandle(handle);
                Err(error)
            }
            unexpected => {
                CloseHandle(handle);
                Err(std::io::Error::other(format!(
                    "unexpected daemon mutex wait result: {unexpected}"
                )))
            }
        }
    }
}

#[cfg(not(debug_assertions))]
fn daemon_mutex_name() -> std::io::Result<Cow<'static, str>> {
    Ok(Cow::Borrowed(DAEMON_MUTEX_NAME))
}

#[cfg(debug_assertions)]
fn daemon_mutex_name() -> std::io::Result<Cow<'static, str>> {
    let namespace = test_namespace::from_env().map_err(invalid_test_namespace)?;
    daemon_mutex_name_for(namespace.as_deref())
        .map(Cow::Owned)
        .map_err(invalid_test_namespace)
}

#[cfg(debug_assertions)]
fn daemon_mutex_name_for(namespace: Option<&str>) -> Result<String, String> {
    test_namespace::qualify(DAEMON_MUTEX_NAME, TEST_MUTEX_SEPARATOR, namespace)
}

#[cfg(debug_assertions)]
fn invalid_test_namespace(error: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{daemon_mutex_name_for, normalize_local_backend_path};

    #[test]
    fn local_backend_paths_use_forward_slashes() {
        assert_eq!(
            normalize_local_backend_path(r"C:\Users\Alice\transfer"),
            "C:/Users/Alice/transfer"
        );
        assert_eq!(
            normalize_local_backend_path(r"\\server\share\transfer"),
            "//server/share/transfer"
        );
        assert_eq!(
            normalize_local_backend_path(r"C:relative\transfer"),
            "C:relative/transfer"
        );
    }

    #[test]
    fn daemon_mutex_names_are_exact_and_isolated() {
        assert_eq!(
            daemon_mutex_name_for(None).unwrap(),
            r"Local\SmartExplorerSyncDaemon"
        );
        assert_eq!(
            daemon_mutex_name_for(Some("device_B2")).unwrap(),
            r"Local\SmartExplorerSyncDaemon.Test.device_B2"
        );
    }

    #[test]
    fn daemon_mutex_rejects_an_unsafe_namespace() {
        assert!(daemon_mutex_name_for(Some(r"device\B")).is_err());
    }
}

#[path = "drives.rs"]
mod drives;
#[path = "volume_monitor.rs"]
mod volume_monitor;
pub(crate) fn drive_snapshot() -> Option<Vec<DriveInfo>> { volume_monitor::snapshot() }

mod power {
    pub fn battery_saver_on() -> bool {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut st = SYSTEM_POWER_STATUS::default();
        unsafe {
            if GetSystemPowerStatus(&mut st).is_ok() {
                // SystemStatusFlag bit0 = "battery saver on" (Windows 10+).
                st.SystemStatusFlag & 0x01 != 0
            } else {
                false
            }
        }
    }

    pub fn on_metered_network() -> bool {
        use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
        // Best-effort via WinRT: treat Fixed/Variable cost as metered. Any error
        // (no connection, API unavailable) is treated as not-metered.
        (|| -> windows::core::Result<bool> {
            let profile = NetworkInformation::GetInternetConnectionProfile()?;
            let cost = profile.GetConnectionCost()?;
            let t = cost.NetworkCostType()?;
            Ok(t == NetworkCostType::Fixed || t == NetworkCostType::Variable)
        })()
        .unwrap_or(false)
    }
}

pub(crate) fn watch_case_fold() -> bool { true }

#[path = "session.rs"]
mod session;
pub(crate) use session::session_marker;
pub(crate) fn daemon_command(executable: &std::path::Path) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new(executable);
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command
}
