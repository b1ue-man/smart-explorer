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
    drives::removable()
}

pub(crate) fn battery_saver_on() -> bool {
    power::battery_saver_on()
}

pub(crate) fn on_metered_network() -> bool {
    power::on_metered_network()
}

pub(crate) fn run_shell_command(cmd: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("cmd").args(["/C", cmd]).status()
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

    let name: Vec<u16> = std::ffi::OsStr::new(r"Local\SmartExplorerSyncDaemon")
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

mod drives {
    use super::DriveInfo;
    use std::os::windows::ffi::OsStrExt;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(Some(0))
            .collect()
    }

    pub fn removable() -> Vec<DriveInfo> {
        use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};
        // GetDriveTypeW returns a plain u32; DRIVE_REMOVABLE == 2.
        const DRIVE_REMOVABLE: u32 = 2;
        let mut out = Vec::new();
        let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let root = format!("{}:\\", letter);
            let rootw = wide(&root);
            let dtype = unsafe { GetDriveTypeW(windows::core::PCWSTR(rootw.as_ptr())) };
            if dtype != DRIVE_REMOVABLE {
                continue;
            }
            let mut name = [0u16; 261];
            let mut serial: u32 = 0;
            let label = unsafe {
                if GetVolumeInformationW(
                    windows::core::PCWSTR(rootw.as_ptr()),
                    Some(&mut name),
                    Some(&mut serial),
                    None,
                    None,
                    None,
                )
                .is_ok()
                {
                    let len = name.iter().position(|&c| c == 0).unwrap_or(0);
                    String::from_utf16_lossy(&name[..len])
                } else {
                    String::new()
                }
            };
            out.push(DriveInfo {
                letter: format!("{}:", letter),
                label,
                serial: format!("{:08X}", serial),
            });
        }
        out
    }
}

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
