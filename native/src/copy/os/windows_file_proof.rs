//! Qualified Windows object incarnations; also included by standalone agents.
use std::{fs::File, io, mem::size_of, os::windows::io::AsRawHandle};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    GetVolumeInformationByHandleW, BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO,
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
    fn NtQueryVolumeInformationFile(
        file: *mut std::ffi::c_void,
        completion: *mut IO_STATUS_BLOCK,
        data: *mut std::ffi::c_void,
        bytes: u32,
        class: i32,
    ) -> i32;
}

/// Simultaneous live handles have a separate, wider compatibility contract
/// from persistent recovery IDs. Never truncate an available ReFS identity.
pub(crate) fn same_file_object(left: &File, right: &File) -> io::Result<bool> {
    #[derive(PartialEq, Eq)]
    enum Live {
        Full(u64, [u8; 16]),
        Legacy(u32, u32, u32),
    }
    fn identity(file: &File) -> io::Result<Live> {
        let mut full: FILE_ID_INFO = unsafe { std::mem::zeroed() };
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut full as *mut FILE_ID_INFO).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        } != 0
        {
            if full.FileId.Identifier == [0; 16] {
                return Err(unsupported("provider has no live file identity"));
            }
            return Ok(Live::Full(full.VolumeSerialNumber, full.FileId.Identifier));
        }
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(1 | 50 | 87 | 124)) {
            return Err(error);
        }
        let mut legacy: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut legacy) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if legacy.nFileIndexHigh == 0 && legacy.nFileIndexLow == 0 {
            return Err(unsupported("provider has no live file identity"));
        }
        Ok(Live::Legacy(
            legacy.dwVolumeSerialNumber,
            legacy.nFileIndexHigh,
            legacy.nFileIndexLow,
        ))
    }
    Ok(identity(left)? == identity(right)?)
}

pub(crate) fn stable_handle_identity(file: &File) -> io::Result<String> {
    qualify_local_volume(file)?;
    let mut name = [0u16; 261];
    // SAFETY: the live handle and complete output buffer remain valid.
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
        return Err(capability_error(io::Error::last_os_error()));
    }
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    // NTFS references contain the MFT sequence number. A generic provider's
    // 128-bit FileIdInfo by itself promises comparison of live handles, not
    // an incarnation after deletion/reuse. ReFS/FAT/network providers without
    // a separately qualified incarnation therefore remain unsupported here.
    if !String::from_utf16_lossy(&name[..length]).eq_ignore_ascii_case("NTFS") {
        return Err(unsupported(
            "filesystem has no qualified persistent file incarnation",
        ));
    }
    let mut identity: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut identity as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(capability_error(io::Error::last_os_error()));
    }
    let bytes = identity.FileId.Identifier;
    // NTFS's full reference is 48-bit MFT segment plus nonzero 16-bit
    // sequence; do not interpret a provider's object-ID shape as that layout.
    if identity.VolumeSerialNumber == 0
        || bytes[..6] == [0; 6]
        || bytes[6..8] == [0; 2]
        || bytes[8..] != [0; 8]
    {
        return Err(unsupported(
            "provider returned no qualified NTFS file reference",
        ));
    }
    let file_id: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "windows-ntfs-file-v2:{}:{file_id}",
        identity.VolumeSerialNumber
    ))
}

fn qualify_local_volume(file: &File) -> io::Result<()> {
    let file = file.try_clone()?;
    let mut data = Box::new([0u32; 2]);
    let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
    // FileFsDeviceInformation: query the held volume, never a drive letter.
    let status = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            completion.as_mut(),
            data.as_mut_ptr().cast(),
            8,
            4,
        )
    };
    let status = if status == STATUS_PENDING {
        if unsafe { WaitForSingleObject(file.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            std::mem::forget((file, data, completion));
            return Err(io::Error::other(
                "file identity volume query completion unconfirmed",
            ));
        }
        let status = unsafe { completion.Anonymous.Status };
        if status == STATUS_PENDING {
            std::mem::forget((file, data, completion));
            return Err(io::Error::other("file identity volume query still pending"));
        }
        status
    } else {
        status
    };
    if status < 0 {
        return Err(capability_error(io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(status)
        }
            as i32)));
    }
    if status != 0 || completion.Information != 8 {
        return Err(unsupported(
            "provider returned no complete volume device facts",
        ));
    }
    if data[1] & (0x10 | 0x2000) != 0 {
        // REMOTE_DEVICE / WEBDAV_DEVICE
        return Err(unsupported(
            "remote volume has no qualified local file incarnation",
        ));
    }
    Ok(())
}

fn unsupported(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message)
}

fn capability_error(error: io::Error) -> io::Error {
    if matches!(error.raw_os_error(), Some(1 | 5 | 50 | 87 | 124)) {
        io::Error::new(io::ErrorKind::Unsupported, error)
    } else {
        error
    }
}
