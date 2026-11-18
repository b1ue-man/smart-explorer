//! The driver's root device (plan 2026-11-17 P5): found by its hardware id
//! `Root\MttVDD` among the display class devices, created as devcon and
//! nefcon do (`SetupDiCreateDeviceInfoW` with a generated id, the hardware
//! id, `DIF_REGISTERDEVICE`), its driver installed with
//! `UpdateDriverForPlugAndPlayDevicesW` after `SetupCopyOEMInfW` staged the
//! package (whose driver store name is kept to remove the package after a
//! newer one replaced it). Reading the state needs no rights; the changes
//! run only in the elevated step.
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_Status, SetupCopyOEMInfW, SetupDiCallClassInstaller,
    SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW,
    SetupDiSetDeviceRegistryPropertyW, SetupUninstallOEMInfW, UpdateDriverForPlugAndPlayDevicesW,
    CR_SUCCESS, DICD_GENERATE_ID, DIF_REGISTERDEVICE, DN_HAS_PROBLEM, DN_STARTED,
    GUID_DEVCLASS_DISPLAY, HDEVINFO, INSTALLFLAG_FORCE, SPDRP_HARDWAREID, SPOST_PATH,
    SP_DEVINFO_DATA,
};
use windows_sys::Win32::Foundation::{GetLastError, BOOL};

use crate::screen::core::virtual_display::windows_driver::HARDWARE_ID;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error(what: &str) -> String {
    // SAFETY: reads the calling thread's last error.
    format!("{what}: Win32 {}", unsafe { GetLastError() })
}

/// A device information set, destroyed on drop.
struct InfoSet(HDEVINFO);

impl InfoSet {
    fn valid(&self) -> bool {
        self.0 != -1 && self.0 != 0
    }
}

impl Drop for InfoSet {
    fn drop(&mut self) {
        if self.valid() {
            // SAFETY: the set was returned by SetupAPI and is destroyed once.
            unsafe { SetupDiDestroyDeviceInfoList(self.0) };
        }
    }
}

fn info_data() -> SP_DEVINFO_DATA {
    SP_DEVINFO_DATA {
        cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
        ClassGuid: GUID_DEVCLASS_DISPLAY,
        DevInst: 0,
        Reserved: 0,
    }
}

/// The hardware ids of one device (a multi-string).
fn hardware_ids(set: &InfoSet, data: &SP_DEVINFO_DATA) -> Vec<String> {
    let mut buffer = vec![0u8; 2048];
    let mut needed = 0u32;
    // SAFETY: buffer and its length match; data belongs to set.
    let read = unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set.0,
            data,
            SPDRP_HARDWAREID,
            null_mut(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut needed,
        )
    };
    if read == 0 {
        return Vec::new();
    }
    let units: Vec<u16> = buffer[..(needed as usize).min(buffer.len())]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    units
        .split(|unit| *unit == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// The devnode of se's driver device, present or not; `None` without one.
pub fn find() -> Option<u32> {
    // SAFETY: plain query of the display class; flags 0 lists every device
    // the registry knows, present or not.
    let set =
        InfoSet(unsafe { SetupDiGetClassDevsW(&GUID_DEVCLASS_DISPLAY, null(), null_mut(), 0) });
    if !set.valid() {
        return None;
    }
    for index in 0.. {
        let mut data = info_data();
        // SAFETY: data has its size set; index walks the set.
        if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut data) } == 0 {
            return None;
        }
        if hardware_ids(&set, &data)
            .iter()
            .any(|id| id.eq_ignore_ascii_case(HARDWARE_ID))
        {
            return Some(data.DevInst);
        }
    }
    None
}

/// Whether se's driver device is started without a problem.
pub fn working() -> bool {
    let Some(devnode) = find() else {
        return false;
    };
    let mut status = 0u32;
    let mut problem = 0u32;
    // SAFETY: plain status query of a devnode of this machine.
    let result = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, devnode, 0) };
    result == CR_SUCCESS && status & DN_STARTED != 0 && status & DN_HAS_PROBLEM == 0
}

/// Creates the root device `Root\MttVDD` unless one exists.
fn ensure_device() -> Result<(), String> {
    if find().is_some() {
        return Ok(());
    }
    // SAFETY: a new empty set of the display class.
    let set = InfoSet(unsafe { SetupDiCreateDeviceInfoList(&GUID_DEVCLASS_DISPLAY, null_mut()) });
    if !set.valid() {
        return Err(last_error("SetupDiCreateDeviceInfoList"));
    }
    let mut data = info_data();
    let class = wide("Display");
    // SAFETY: strings are NUL-terminated and outlive the call.
    let created = unsafe {
        SetupDiCreateDeviceInfoW(
            set.0,
            class.as_ptr(),
            &GUID_DEVCLASS_DISPLAY,
            null(),
            null_mut(),
            DICD_GENERATE_ID,
            &mut data,
        )
    };
    if created == 0 {
        return Err(last_error("SetupDiCreateDeviceInfoW"));
    }
    // A multi-string: the id, its NUL and the closing NUL.
    let ids: Vec<u8> = HARDWARE_ID
        .encode_utf16()
        .chain([0, 0])
        .flat_map(u16::to_le_bytes)
        .collect();
    // SAFETY: the buffer and its length match.
    let set_id = unsafe {
        SetupDiSetDeviceRegistryPropertyW(
            set.0,
            &mut data,
            SPDRP_HARDWAREID,
            ids.as_ptr(),
            ids.len() as u32,
        )
    };
    if set_id == 0 {
        return Err(last_error("SetupDiSetDeviceRegistryPropertyW"));
    }
    // SAFETY: registers the device just described.
    if unsafe { SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set.0, &data) } == 0 {
        return Err(last_error("DIF_REGISTERDEVICE"));
    }
    Ok(())
}

/// Stages the package of `inf` in the driver store; its store name
/// (`oem12.inf`).
fn stage(inf: &Path) -> Result<String, String> {
    let path = wide(&inf.to_string_lossy());
    let mut name = vec![0u16; 1024];
    let mut needed = 0u32;
    let mut component: windows_sys::core::PWSTR = null_mut();
    // SAFETY: buffers and lengths match; the source path is NUL-terminated.
    let staged = unsafe {
        SetupCopyOEMInfW(
            path.as_ptr(),
            null(),
            SPOST_PATH,
            0,
            name.as_mut_ptr(),
            name.len() as u32,
            &mut needed,
            &mut component,
        )
    };
    if staged == 0 {
        return Err(last_error("SetupCopyOEMInfW (Treiberpaket)"));
    }
    let end = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    let full = String::from_utf16_lossy(&name[..end]);
    Ok(full.rsplit('\\').next().unwrap_or(&full).to_string())
}

/// Installs the package of `inf` on se's device (created if missing); the
/// driver store name of the package.
pub fn install(inf: &Path) -> Result<String, String> {
    let oem = stage(inf)?;
    ensure_device()?;
    let id = wide(HARDWARE_ID);
    let path = wide(&inf.to_string_lossy());
    let mut reboot: BOOL = 0;
    // SAFETY: strings are NUL-terminated; no window owns the call.
    let updated = unsafe {
        UpdateDriverForPlugAndPlayDevicesW(
            null_mut(),
            id.as_ptr(),
            path.as_ptr(),
            INSTALLFLAG_FORCE,
            &mut reboot,
        )
    };
    if updated == 0 {
        return Err(last_error("UpdateDriverForPlugAndPlayDevicesW"));
    }
    Ok(oem)
}

/// Removes a former package from the driver store; a package still in use
/// stays (no force).
pub fn remove_package(oem: &str) -> Result<(), String> {
    let lower = oem.to_ascii_lowercase();
    if !(lower.starts_with("oem") && lower.ends_with(".inf") && !oem.contains(['\\', '/'])) {
        return Err(format!("{oem} ist kein Name des Treiberspeichers"));
    }
    let name = wide(oem);
    // SAFETY: the name is NUL-terminated.
    if unsafe { SetupUninstallOEMInfW(name.as_ptr(), 0, null()) } == 0 {
        return Err(last_error("SetupUninstallOEMInfW"));
    }
    Ok(())
}
