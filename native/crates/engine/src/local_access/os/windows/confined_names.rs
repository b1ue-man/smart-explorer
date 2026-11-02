//! Names only, queried from the held directory object without a path fallback.
use crate::local_access::LocalEntry;
use std::{
    ffi::{c_void, OsString},
    fs::File,
    io,
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{RtlNtStatusToDosError, STATUS_PENDING, WAIT_OBJECT_0},
    System::{
        Threading::{WaitForSingleObject, INFINITE},
        IO::IO_STATUS_BLOCK,
    },
};
#[link(name = "ntdll")]
extern "system" {
    fn NtQueryDirectoryFile(
        file: *mut c_void,
        event: *mut c_void,
        apc: *const c_void,
        context: *mut c_void,
        completion: *mut IO_STATUS_BLOCK,
        data: *mut c_void,
        length: u32,
        class: i32,
        single: u8,
        pattern: *const c_void,
        restart: u8,
    ) -> i32;
}
pub(super) fn read(directory: &File) -> io::Result<Vec<LocalEntry>> {
    let file = directory.try_clone()?;
    let mut entries = Vec::new();
    let mut restart = true;
    loop {
        // A native UNICODE_STRING name cannot exceed 65534 bytes. This holds
        // even its longest complete FILE_NAMES_INFORMATION record.
        let mut data = vec![0u32; 128 * 1024 / 4];
        let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
        let status = unsafe {
            NtQueryDirectoryFile(
                file.as_raw_handle(),
                ptr::null_mut(),
                ptr::null(),
                ptr::null_mut(),
                completion.as_mut(),
                data.as_mut_ptr().cast(),
                (data.len() * 4) as u32,
                12,
                0,
                ptr::null(),
                u8::from(restart),
            )
        };
        restart = false;
        let status = if status == STATUS_PENDING {
            if unsafe { WaitForSingleObject(file.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
                std::mem::forget((file, data, completion));
                return Err(io::Error::other("directory query completion unconfirmed"));
            }
            let status = unsafe { completion.Anonymous.Status };
            if status == STATUS_PENDING {
                std::mem::forget((file, data, completion));
                return Err(io::Error::other("directory query still pending"));
            }
            status
        } else {
            status
        };
        if status as u32 == 0x8000_0006 {
            return Ok(entries);
        } // STATUS_NO_MORE_FILES
        if status < 0 {
            return Err(io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(status) } as i32,
            ));
        }
        let length = completion.Information;
        if length < 12 || length > data.len() * 4 {
            return Err(invalid());
        }
        let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), length) };
        let mut offset = 0usize;
        loop {
            let header = bytes
                .get(offset..offset.checked_add(12).ok_or_else(invalid)?)
                .ok_or_else(invalid)?;
            let next = u32::from_ne_bytes(header[..4].try_into().map_err(|_| invalid())?) as usize;
            let name_length =
                u32::from_ne_bytes(header[8..12].try_into().map_err(|_| invalid())?) as usize;
            if name_length == 0 || name_length % 2 != 0 {
                return Err(invalid());
            }
            let end = offset
                .checked_add(12)
                .and_then(|start| start.checked_add(name_length))
                .ok_or_else(invalid)?;
            let name = bytes.get(offset + 12..end).ok_or_else(invalid)?;
            let wide = name
                .chunks_exact(2)
                .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>();
            let name = OsString::from_wide(&wide);
            if name != "." && name != ".." {
                entries.push(LocalEntry {
                    name,
                    ..LocalEntry::default()
                });
            }
            if next == 0 {
                break;
            }
            if next % 4 != 0 || next < 12 + name_length {
                return Err(invalid());
            }
            offset = offset
                .checked_add(next)
                .filter(|offset| *offset < length)
                .ok_or_else(invalid)?;
        }
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid held directory name record",
    )
}
