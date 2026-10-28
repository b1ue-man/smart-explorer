//! Volume aliases resolve to one storage device number; virtual CD images do
//! not prove a physical drive. All probes execute on the caller's worker.
use super::Source;
use std::{
    fs::File,
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Storage::FileSystem::*,
    System::IO::{DeviceIoControl, OVERLAPPED},
};

#[repr(C)]
#[derive(Default)]
struct DeviceNumber {
    kind: u32,
    number: u32,
    partition: u32,
}

pub(super) fn probe(path: &Path) -> io::Result<Option<Source>> {
    let normalized = crate::local_access::normalize_scan_root(path);
    let text = normalized.to_string_lossy();
    if text.starts_with(r"\\?\UNC\") || (text.starts_with(r"\\") && !text.starts_with(r"\\?\")) {
        return Ok(None);
    }
    let path = wide(&normalized);
    let mut root = vec![0u16; 32768];
    if unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { GetDriveTypeW(root.as_ptr()) } != 5 {
        return Ok(None);
    }
    let mut volume = vec![0u16; 1024];
    if unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), volume.as_mut_ptr(), volume.len() as u32)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let length = volume.iter().position(|c| *c == 0).unwrap_or(volume.len());
    let volume_text = String::from_utf16_lossy(&volume[..length]);
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(volume_text.trim_end_matches('\\'))?;
    probe_volume(&file, &root)
}

/// `probe` of a regular file: a file is never a mount point, so its folder
/// decides. A folder found on no optical volume a moment ago is not
/// resolved again (M6); positive answers are always resolved in full.
pub(super) fn probe_regular_file(path: &Path) -> io::Result<Option<Source>> {
    let normalized = crate::local_access::normalize_scan_root(path);
    let Some(key) = normalized
        .parent()
        .map(|folder| folder.to_string_lossy().to_lowercase())
    else {
        return probe(path);
    };
    if super::probe_cache::known_plain(&key) {
        return Ok(None);
    }
    let found = probe(path)?;
    if found.is_none() {
        super::probe_cache::remember_plain(key);
    }
    Ok(found)
}

pub(super) fn probe_file(file: &File) -> io::Result<Option<Source>> {
    // The handle decides the volume even when the retained root was renamed
    // or its old external spelling now names a different directory.
    let mut path = vec![0u16; 32768];
    let size = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            path.as_mut_ptr(),
            path.len() as u32,
            0x9,
        )
    };
    if size == 0 {
        let error = io::Error::last_os_error();
        // Network shares have no volume GUID; they retain their remote path.
        if error.raw_os_error() == Some(3) {
            return Ok(None);
        }
        return Err(error);
    }
    if size as usize >= path.len() {
        return Err(io::Error::other("held volume path exceeds OS buffer"));
    }
    let text = String::from_utf16_lossy(&path[..size as usize]);
    #[cfg(test)]
    {
        let mut dos = vec![0u16; 32768];
        let size = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle(), dos.as_mut_ptr(), dos.len() as u32, 0)
        };
        if size > 0 && (size as usize) < dos.len() {
            if let Some(source) = super::test_profiles::probe(Path::new(&String::from_utf16_lossy(
                &dos[..size as usize],
            ))) {
                return Ok(Some(source));
            }
        }
    }
    let Some(end) = text.find("}\\") else {
        return Ok(None);
    };
    let root = wide(Path::new(&text[..end + 2]));
    if unsafe { GetDriveTypeW(root.as_ptr()) } != 5 {
        return Ok(None);
    }
    let volume = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(text[..end + 1].to_string())?;
    let source = probe_volume(&volume, &root)?;
    // Compare the pinned file's volume serial with the queried device volume.
    if let Some(source) = &source {
        let mut serial = 0u32;
        if unsafe {
            GetVolumeInformationByHandleW(
                file.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                &mut serial,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if !source.generation.starts_with(&format!("{serial:08x}:")) {
            return Err(io::Error::other("held optical volume changed"));
        }
    }
    Ok(source)
}

fn probe_volume(file: &File, root: &[u16]) -> io::Result<Option<Source>> {
    let mut number = DeviceNumber::default();
    ioctl(&file, 0x002d1080, std::ptr::null(), 0, &mut number)?;
    if number.kind != 2 {
        return Ok(None);
    } // FILE_DEVICE_CD_ROM
      // StorageDeviceProperty / PropertyStandardQuery. BusType is at byte 28
      // of STORAGE_DEVICE_DESCRIPTOR. Unknown, virtual and file-backed virtual
      // buses cannot establish a physical optical source.
    let query = [0u32; 3];
    let mut descriptor = [0u8; 1024];
    ioctl(
        &file,
        0x002d1400,
        query.as_ptr().cast(),
        12,
        &mut descriptor,
    )?;
    let bus = u32::from_le_bytes(descriptor[28..32].try_into().map_err(io::Error::other)?);
    if !(1..=13).contains(&bus) {
        return Ok(None);
    }
    let mut change = 0u32;
    ioctl(&file, 0x002d0800, std::ptr::null(), 0, &mut change)?; // CHECK_VERIFY2
    let mut serial = 0u32;
    let mut flags = 0u32;
    if unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            &mut serial,
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Some(Source {
        device: format!("windows-optical:{}:{}", number.kind, number.number),
        generation: format!("{serial:08x}:{change}:process:{}", process_epoch()?),
        read_only: flags & 0x0008_0000 != 0, // FILE_READ_ONLY_VOLUME
    }))
}

fn ioctl<T>(
    file: &File,
    code: u32,
    input: *const std::ffi::c_void,
    input_len: u32,
    output: &mut T,
) -> io::Result<()> {
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            code,
            input,
            input_len,
            (output as *mut T).cast(),
            std::mem::size_of::<T>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if returned < std::mem::size_of::<T>().min(36) as u32 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "short optical device descriptor",
        ));
    }
    Ok(())
}

pub(super) fn try_lock(file: &File, exclusive: bool) -> io::Result<bool> {
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let flags = LOCKFILE_FAIL_IMMEDIATELY
        | if exclusive {
            LOCKFILE_EXCLUSIVE_LOCK
        } else {
            0
        };
    if unsafe { LockFileEx(file.as_raw_handle(), flags, 0, 1, 0, &mut overlapped) } != 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(33) {
        Ok(false)
    } else {
        Err(error)
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub(super) fn unlock(file: &File) -> io::Result<()> {
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    if unsafe { UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut overlapped) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

// A storage change count is not a persisted media proof across Windows boots.
// Conservatively certify reuse only within this actual process lifetime.
fn process_epoch() -> io::Result<&'static str> {
    static EPOCH: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    match EPOCH.get_or_init(|| {
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).map_err(|error| error.to_string())?;
        Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
    }) {
        Ok(epoch) => Ok(epoch),
        Err(error) => Err(io::Error::other(error.clone())),
    }
}
