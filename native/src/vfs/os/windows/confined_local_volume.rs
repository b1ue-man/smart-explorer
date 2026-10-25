//! Volume facts from the live directory handle, independent of drive-letter reuse.
use std::{ffi::c_void, fs::File, io, os::windows::io::AsRawHandle};
use windows_sys::Win32::{Foundation::{RtlNtStatusToDosError, STATUS_PENDING, WAIT_OBJECT_0}, Storage::FileSystem::{GetVolumeInformationByHandleW, GetFinalPathNameByHandleW, GetFileInformationByHandleEx, FileIdInfo, FILE_ID_INFO, VOLUME_NAME_NONE}, System::{IO::IO_STATUS_BLOCK, Threading::{WaitForSingleObject, INFINITE}}};
#[link(name = "ntdll")]
extern "system" {
    fn NtQueryVolumeInformationFile(file: *mut c_void, completion: *mut IO_STATUS_BLOCK, data: *mut c_void, length: u32, class: i32) -> i32;
}

pub(super) fn query(directory: &File, class: i32, bytes: usize) -> io::Result<Vec<u8>> {
    let file = directory.try_clone()?;
    let mut data = vec![0u64; bytes.div_ceil(8)];
    let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
    let status = unsafe { NtQueryVolumeInformationFile(file.as_raw_handle(), completion.as_mut(), data.as_mut_ptr().cast(), bytes as u32, class) };
    let status = if status == STATUS_PENDING {
        if unsafe { WaitForSingleObject(file.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            std::mem::forget((file, data, completion));
            return Err(io::Error::other("volume query completion unconfirmed"));
        }
        let status = unsafe { completion.Anonymous.Status };
        if status == STATUS_PENDING {
            std::mem::forget((file, data, completion));
            return Err(io::Error::other("volume query still pending"));
        }
        status
    } else { status };
    if status < 0 { return Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)); }
    if completion.Information != bytes { return Err(io::Error::new(io::ErrorKind::InvalidData, "volume query returned an incomplete record")); }
    Ok(data.into_iter().flat_map(u64::to_ne_bytes).take(bytes).collect())
}

pub(super) fn facts(file: &File) -> io::Result<(String, u32, u64, u32)> {
    let (mut serial, mut maximum, mut flags) = (0, 0, 0);
    let mut name = [0u16; 261];
    if unsafe { GetVolumeInformationByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0, &mut serial, &mut maximum, &mut flags, name.as_mut_ptr(), name.len() as u32) } == 0 { return Err(io::Error::last_os_error()); }
    let mut id: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    let serial = if unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo, (&mut id as *mut FILE_ID_INFO).cast(), std::mem::size_of::<FILE_ID_INFO>() as u32) } != 0 { id.VolumeSerialNumber } else { u64::from(serial) };
    let length = name.iter().position(|unit| *unit == 0).unwrap_or(name.len());
    Ok((String::from_utf16_lossy(&name[..length]), maximum, serial, flags))
}

pub(super) fn inside(file: &File) -> io::Result<String> {
    let mut data = vec![0u16; 512];
    loop {
        let length = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), data.as_mut_ptr(), data.len() as u32, VOLUME_NAME_NONE) } as usize;
        if length == 0 { return Err(io::Error::last_os_error()); }
        if length < data.len() { return Ok(String::from_utf16_lossy(&data[..length]).replace('\\', "/").trim_start_matches('/').into()); }
        if length >= 32768 { return Err(io::Error::new(io::ErrorKind::InvalidData, "volume-relative path too long")); }
        data.resize(length + 1, 0);
    }
}
