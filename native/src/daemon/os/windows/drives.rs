//! USB/SD/FireWire volumes, including DRIVE_FIXED USB HDDs and directory
//! mount points. All probing is done on the volume monitor's thread.
use super::DriveInfo;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetVolumeInformationW, GetVolumePathNamesForVolumeNameW, BusTypeUsb, BusTypeSd, BusTypeMmc, BusType1394, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE, OPEN_EXISTING};
use windows_sys::Win32::System::Ioctl::{IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, StorageDeviceProperty, STORAGE_PROPERTY_QUERY, STORAGE_DEVICE_DESCRIPTOR};
use windows_sys::Win32::System::IO::DeviceIoControl;

pub(super) fn removable() -> Vec<DriveInfo> {
    let mut previous = 0;
    unsafe { windows_sys::Win32::System::Diagnostics::Debug::SetThreadErrorMode(1 | 0x8000, &mut previous); }
    let result = enumerate();
    unsafe { windows_sys::Win32::System::Diagnostics::Debug::SetThreadErrorMode(previous, std::ptr::null_mut()); }
    result
}
fn enumerate() -> Vec<DriveInfo> {
    let mut volume = vec![0u16; 1024];
    let enumeration = unsafe { FindFirstVolumeW(volume.as_mut_ptr(), volume.len() as u32) };
    if enumeration == INVALID_HANDLE_VALUE { return Vec::new(); }
    let mut result = Vec::new();
    loop {
        let end = volume.iter().position(|unit| *unit == 0).unwrap_or(volume.len());
        let name = String::from_utf16_lossy(&volume[..end]);
        if external(&name) {
            let mut label = [0u16; 261]; let mut serial = 0;
            if unsafe { GetVolumeInformationW(volume.as_ptr(), label.as_mut_ptr(), label.len() as u32,
                &mut serial, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0) } != 0 {
                let label = String::from_utf16_lossy(&label[..label.iter().position(|unit| *unit == 0).unwrap_or(label.len())]);
                for path in paths(&volume) {
                    let root = if path.len() == 3 && path.ends_with('\\') { path[..2].to_string() } else { path };
                    result.push(DriveInfo { letter: root, label: label.clone(), serial: format!("{name}:{serial:08X}") });
                }
            }
        }
        if unsafe { FindNextVolumeW(enumeration, volume.as_mut_ptr(), volume.len() as u32) } == 0 { break; }
    }
    unsafe { FindVolumeClose(enumeration); }
    result
}
fn external(volume: &str) -> bool {
    let path: Vec<u16> = volume.trim_end_matches('\\').encode_utf16().chain(Some(0)).collect();
    let handle = unsafe { CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
    if handle == INVALID_HANDLE_VALUE { return false; }
    let mut query: STORAGE_PROPERTY_QUERY = unsafe { std::mem::zeroed() };
    query.PropertyId = StorageDeviceProperty; query.QueryType = PropertyStandardQuery;
    let mut buffer = [0u64; 512]; let mut returned = 0;
    let success = unsafe { DeviceIoControl(handle, IOCTL_STORAGE_QUERY_PROPERTY,
        (&query as *const STORAGE_PROPERTY_QUERY).cast(), std::mem::size_of_val(&query) as u32,
        buffer.as_mut_ptr().cast(), std::mem::size_of_val(&buffer) as u32, &mut returned, std::ptr::null_mut()) } != 0;
    unsafe { CloseHandle(handle); }
    if !success || returned < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() as u32 { return false; }
    let descriptor: STORAGE_DEVICE_DESCRIPTOR = unsafe { std::ptr::read_unaligned(buffer.as_ptr().cast()) };
    matches!(descriptor.BusType, BusTypeUsb | BusTypeSd | BusTypeMmc | BusType1394)
}
fn paths(volume: &[u16]) -> Vec<String> {
    let mut paths = vec![0u16; 4096]; let mut needed = 0;
    let mut success = unsafe { GetVolumePathNamesForVolumeNameW(volume.as_ptr(), paths.as_mut_ptr(), paths.len() as u32, &mut needed) };
    if success == 0 && needed > paths.len() as u32 && needed <= 1024 * 1024 {
        paths.resize(needed as usize, 0);
        success = unsafe { GetVolumePathNamesForVolumeNameW(volume.as_ptr(), paths.as_mut_ptr(), paths.len() as u32, &mut needed) };
    }
    if success == 0 { return Vec::new(); }
    paths.split(|unit| *unit == 0).take_while(|part| !part.is_empty()).map(String::from_utf16_lossy).collect()
}
