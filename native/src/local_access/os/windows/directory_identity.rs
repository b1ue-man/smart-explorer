//! Compare directory pins without truncating a provider's 128-bit identity.
use std::{fs::File, io, mem::size_of, os::windows::io::AsRawHandle};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO,
};

#[derive(PartialEq, Eq)]
enum Identity {
    Full(u64, [u8; 16]),
    Legacy(u32, u32, u32),
}

fn identity(file: &File) -> io::Result<Identity> {
    let mut full: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo,
            (&mut full as *mut FILE_ID_INFO).cast(), size_of::<FILE_ID_INFO>() as u32)
    } != 0 {
        if full.FileId.Identifier == [0; 16] {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "provider has no directory identity"));
        }
        return Ok(Identity::Full(full.VolumeSerialNumber, full.FileId.Identifier));
    }
    let error = io::Error::last_os_error();
    if !matches!(error.raw_os_error(), Some(1 | 50 | 87 | 124)) {
        return Err(error);
    }
    // Older/FAT providers retain their documented 64-bit identity. Do not
    // silently truncate an available ReFS/NTFS 128-bit ID.
    let mut legacy: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut legacy) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if legacy.nFileIndexHigh == 0 && legacy.nFileIndexLow == 0 {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "provider has no directory identity"));
    }
    Ok(Identity::Legacy(legacy.dwVolumeSerialNumber, legacy.nFileIndexHigh, legacy.nFileIndexLow))
}

pub(super) fn same_object(left: &File, right: &File) -> io::Result<bool> {
    Ok(identity(left)? == identity(right)?)
}
