#[derive(Clone)]
pub struct DriveInfo {
    pub letter: String,
    pub label: String,
    pub serial: String,
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
