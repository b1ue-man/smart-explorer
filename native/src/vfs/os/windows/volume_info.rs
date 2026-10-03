//! Volume facts of a local Windows location: 64-bit volume serial number,
//! filesystem name and name limit, and the location inside the volume.
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, GetFileInformationByHandleEx, GetFinalPathNameByHandleW,
    GetVolumeInformationByHandleW, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, VOLUME_NAME_NONE,
};

/// `MAX_PATH + 1` UTF-16 units, the documented filesystem-name buffer size.
const NAME_BUFFER: usize = 261;

struct QuietVolumeProbe(u32);

impl QuietVolumeProbe {
    fn enter() -> io::Result<Self> {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            GetThreadErrorMode, SetThreadErrorMode, SEM_FAILCRITICALERRORS,
        };
        // SAFETY: changes only the calling thread; preserve its other flags.
        let current = unsafe { GetThreadErrorMode() };
        let mut previous = 0;
        if unsafe { SetThreadErrorMode(current | SEM_FAILCRITICALERRORS, &mut previous) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(previous))
    }
}

impl Drop for QuietVolumeProbe {
    fn drop(&mut self) {
        // SAFETY: restore the mode on the same synchronous calling thread.
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::SetThreadErrorMode(
                self.0,
                std::ptr::null_mut(),
            );
        }
    }
}

pub(crate) struct VolumeFacts {
    /// `FILE_ID_INFO` serial number (32-bit volume serial where unavailable).
    pub(crate) serial: u64,
    pub(crate) fs_name: String,
    pub(crate) max_component: u32,
    /// Location inside the volume, forward slashes, no leading slash.
    pub(crate) inside: String,
}

/// Facts of the volume that stores `path` (links on the way are followed).
pub(crate) fn volume_facts(path: &Path) -> io::Result<VolumeFacts> {
    let _quiet = QuietVolumeProbe::enter()?;
    let mut current = path;
    let mut missing = Vec::new();
    let file = loop {
        let opened = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(crate::local_access::normalize_scan_root(current));
        match opened {
            Ok(file) => break file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let (Some(parent), Some(name)) = (current.parent(), current.file_name()) else {
                    return Err(error);
                };
                missing.push(name.to_os_string());
                current = parent;
            }
            Err(error) => return Err(error),
        }
    };
    let mut serial = 0u32;
    let mut max_component = 0u32;
    let mut flags = 0u32;
    let mut name = [0u16; NAME_BUFFER];
    // SAFETY: the handle stays open (owned by `file`); every out pointer
    // refers to a live local and the name buffer's length is passed.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            &mut serial,
            &mut max_component,
            &mut flags,
            name.as_mut_ptr(),
            NAME_BUFFER as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(NAME_BUFFER);
    let mut inside = PathBuf::from(inside_volume(&file)?);
    inside.extend(missing.iter().rev());
    Ok(VolumeFacts {
        serial: file_id_serial(&file).unwrap_or(u64::from(serial)),
        fs_name: String::from_utf16_lossy(&name[..length]),
        max_component,
        inside: inside.to_string_lossy().replace('\\', "/"),
    })
}

/// The 64-bit serial number of `FILE_ID_INFO` (Windows 8+, SMB 3, ReFS).
fn file_id_serial(file: &std::fs::File) -> Option<u64> {
    // SAFETY: FILE_ID_INFO is plain data for which zero is a valid value.
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: the buffer is a live FILE_ID_INFO of exactly the passed size.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    (ok != 0).then_some(info.VolumeSerialNumber)
}

/// The handle's final path relative to its volume (`VOLUME_NAME_NONE`):
/// drive letters, mount folders and junctions on the way do not change it.
fn inside_volume(file: &std::fs::File) -> io::Result<String> {
    let mut buffer = vec![0u16; 512];
    loop {
        // SAFETY: the buffer is live and its length is passed.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                VOLUME_NAME_NONE,
            )
        } as usize;
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length < buffer.len() {
            let path = String::from_utf16_lossy(&buffer[..length]);
            return Ok(path.replace('\\', "/").trim_start_matches('/').to_string());
        }
        // Too small: `length` is the size needed including the terminator.
        buffer.resize(length + 1, 0);
    }
}
