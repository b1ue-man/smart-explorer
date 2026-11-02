//! Qualify local namespace observations after an unsuccessful publication.
use std::{ffi::c_void, fs::File, io, os::windows::io::AsRawHandle};
use windows_sys::Win32::{
    Foundation::{RtlNtStatusToDosError, STATUS_PENDING, WAIT_OBJECT_0},
    Storage::FileSystem::GetVolumeInformationByHandleW,
    System::{
        Threading::{WaitForSingleObject, INFINITE},
        IO::IO_STATUS_BLOCK,
    },
};

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryVolumeInformationFile(
        file: *mut c_void,
        completion: *mut IO_STATUS_BLOCK,
        data: *mut c_void,
        bytes: u32,
        class: i32,
    ) -> i32;
}

pub(super) fn require_local_publication_observation(parent: &File) -> io::Result<()> {
    let file = parent.try_clone()?;
    let mut device = Box::new([0u32; 2]);
    let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
    let status = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            completion.as_mut(),
            device.as_mut_ptr().cast(),
            8,
            4,
        )
    };
    let status = if status == STATUS_PENDING {
        if unsafe { WaitForSingleObject(file.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            std::mem::forget((file, device, completion));
            return Err(unknown());
        }
        let status = unsafe { completion.Anonymous.Status };
        if status == STATUS_PENDING {
            std::mem::forget((file, device, completion));
            return Err(unknown());
        }
        status
    } else {
        status
    };
    if status < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    if status != 0 || completion.Information != 8 || device[1] & (0x10 | 0x2000) != 0 {
        return Err(unknown());
    }
    let mut name = [0u16; 261];
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    let name = String::from_utf16_lossy(&name[..length]);
    if ["NTFS", "ReFS", "FAT", "FAT32", "exFAT"]
        .iter()
        .any(|known| name.eq_ignore_ascii_case(known))
    {
        Ok(())
    } else {
        Err(unknown())
    }
}

fn unknown() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "provider cannot establish nonpublication from local namespace observations",
    )
}
