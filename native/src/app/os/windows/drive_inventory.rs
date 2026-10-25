//! Volume identity is independent of its drive-letter presentation.
use crate::app::drive_snapshot::DriveEntry;
use windows_sys::Win32::Storage::FileSystem::*;

pub(crate) fn read_drive_inventory() -> Result<Vec<DriveEntry>, String> {
    let _mode = suppress_media_errors();
    let bits = unsafe { GetLogicalDrives() };
    if bits == 0 { return Err(std::io::Error::last_os_error().to_string()); }
    let mut entries = Vec::new();
    for i in 0..26 {
        if bits & (1 << i) == 0 { continue; }
        let root = format!("{}:\\", char::from(b'A' + i as u8));
        let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
        let mut volume = [0u16; 1024];
        let mut serial = 0u32;
        // QueryDosDevice reads the local DOS namespace, without contacting a
        // mapped server. Unknown redirectors stay Unknown rather than probing it.
        let target = dos_target(&root);
        let local = target.as_deref().is_some_and(|target|
            target.starts_with("\\Device\\HarddiskVolume") || target.starts_with("\\Device\\CdRom"));
        if !local {
            let identity = target.as_deref().and_then(mapped_identity);
            entries.push(DriveEntry { root, incarnation: identity.clone(), identity, free: 0, total: 0 });
            continue;
        }
        let volume_ok = unsafe { GetVolumeNameForVolumeMountPointW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) } != 0;
        let media_ok = unsafe { GetVolumeInformationW(wide.as_ptr(), std::ptr::null_mut(), 0,
            &mut serial, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0) } != 0;
        let identity = if volume_ok && media_ok {
            Some(format!("volume:{}:{serial:08x}", wide_string(&volume)))
        } else { None };
        entries.push(DriveEntry { root, incarnation: identity.clone(), identity, free: 0, total: 0 });
    }
    Ok(entries)
}

fn wide_string(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|c| *c == 0).unwrap_or(value.len())])
}

fn dos_target(root: &str) -> Option<String> {
    let local: Vec<u16> = root.trim_end_matches('\\').encode_utf16().chain(Some(0)).collect();
    let mut target = [0u16; 32768];
    let size = unsafe { QueryDosDeviceW(local.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
    (size != 0).then(|| wide_string(&target))
}

fn mapped_identity(target: &str) -> Option<String> {
    let remote = if let Some(rest) = target.strip_prefix("\\Device\\LanmanRedirector\\") {
        if rest.starts_with(';') { rest.split_once('\\')?.1 } else { rest }
    } else {
        let rest = target.strip_prefix("\\Device\\Mup\\")?;
        if rest.starts_with(';') { return None; }
        rest
    };
    let (server, share) = remote.split_once('\\')?;
    if server.is_empty() || share.is_empty() { return None; }
    // Discard the local logon/connection token, preserve the actual UNC target.
    Some(format!("mapped:{server}\\{share}").to_lowercase())
}

pub(crate) fn read_drive_capacity(root: &str) -> (u64, u64) {
    let _mode = suppress_media_errors();
    let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
    let mut free = 0;
    let mut total = 0;
    let mut total_free = 0;
    unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free); }
    (free, total)
}

fn suppress_media_errors() -> ErrorMode {
    use windows_sys::Win32::System::Diagnostics::Debug::{SetThreadErrorMode, SEM_FAILCRITICALERRORS};
    let mut previous = 0;
    let changed = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, &mut previous) } != 0;
    ErrorMode(changed.then_some(previous))
}

struct ErrorMode(Option<u32>);
impl Drop for ErrorMode {
    fn drop(&mut self) {
        if let Some(previous) = self.0 {
            unsafe { windows_sys::Win32::System::Diagnostics::Debug::SetThreadErrorMode(previous, std::ptr::null_mut()); }
        }
    }
}
