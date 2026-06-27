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

pub(crate) fn acquire_daemon_instance_guard(
    timeout: std::time::Duration,
) -> Option<DaemonInstanceGuard> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match try_acquire_daemon_mutex() {
            Ok(Some(guard)) => return Some(guard),
            Ok(None) if super::state::stop_requested() && std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            Ok(None) => return None,
            Err(e) => {
                super::state::log(&format!("daemon single-instance lock failed: {e}"));
                return None;
            }
        }
    }
}

fn try_acquire_daemon_mutex() -> std::io::Result<Option<DaemonInstanceGuard>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = std::ffi::OsStr::new(r"Local\SmartExplorerSyncDaemon")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null_mut(), 1, name.as_ptr());
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(handle);
            return Ok(None);
        }
        Ok(Some(DaemonInstanceGuard(handle)))
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
