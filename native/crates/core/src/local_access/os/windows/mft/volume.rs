//! Windows FFI of the MFT reader: volume identity queries, the raw volume
//! handle, the two FSCTLs and the aligned, unbuffered `$MFT` reads. All
//! parsing happens in `local_access::mft` on the bytes read here.
use crate::local_access::mft::{
    parse_retrieval_pointers, parse_volume_data, plan_reads, MftExtent, MftIndex, MftRecordReader,
    NtfsVolumeData, VOLUME_DATA_LEN,
};
use std::{
    ffi::{c_void, OsStr},
    fs::{File, OpenOptions},
    io,
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        fs::{FileExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr::{null, null_mut},
    sync::atomic::Ordering,
};
use windows_sys::Win32::{
    Foundation::ERROR_MORE_DATA,
    Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
    Storage::FileSystem::{
        GetDriveTypeW, GetFileInformationByHandle, GetVolumeInformationW,
        GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, BY_HANDLE_FILE_INFORMATION,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
    System::{
        Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, FSCTL_GET_RETRIEVAL_POINTERS},
        Threading::{GetCurrentProcess, OpenProcessToken},
        IO::DeviceIoControl,
    },
};

use super::MftControl;

/// `SYNCHRONIZE` access right: synchronous I/O on the handles below.
const SYNCHRONIZE: u32 = 0x0010_0000;
const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
/// One unbuffered read; a multiple of every sector and record size.
const CHUNK: u64 = 4 * 1024 * 1024;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
/// `GENERIC_READ | GENERIC_WRITE`: `FlushFileBuffers` needs write access.
const GENERIC_READ_WRITE: u32 = 0x8000_0000 | 0x4000_0000;
/// Longest volume path / GUID name the queries return (`MAX_PATH`).
const NAME_CAPACITY: usize = 261;

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

fn until_nul(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// A `len`-byte window of `storage` aligned to `align` (a power of two).
fn aligned(storage: &mut Vec<u8>, align: usize, len: usize) -> io::Result<&mut [u8]> {
    storage.resize(len + align, 0);
    let shift = storage.as_ptr().align_offset(align);
    storage
        .get_mut(shift..shift.saturating_add(len))
        .ok_or_else(|| io::Error::other("Puffer nicht ausrichtbar"))
}

pub(super) fn process_elevated() -> io::Result<bool> {
    let mut handle = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken returned a fresh handle this value now owns.
    let token = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut written = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast::<c_void>(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut written,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(elevation.TokenIsElevated != 0)
}

/// The mount point (`C:\`, `C:\mnt\disk\`, ...) of the volume holding `path`.
pub(super) fn volume_path(path: &OsStr) -> io::Result<String> {
    let input = wide(path);
    let mut output = vec![0u16; NAME_CAPACITY];
    let ok =
        unsafe { GetVolumePathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(until_nul(&output))
}

/// Whether `mount` (with trailing backslash) is a local fixed or removable drive.
pub(super) fn local_drive(mount: &str) -> bool {
    let root = wide(OsStr::new(mount));
    matches!(
        unsafe { GetDriveTypeW(root.as_ptr()) },
        DRIVE_FIXED | DRIVE_REMOVABLE
    )
}

/// File system name and 32-bit serial of the volume mounted at `mount`.
pub(super) fn file_system(mount: &str) -> io::Result<(String, u32)> {
    let root = wide(OsStr::new(mount));
    let mut serial = 0u32;
    let mut name = vec![0u16; NAME_CAPACITY];
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            null_mut(),
            0,
            &mut serial,
            null_mut(),
            null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((until_nul(&name), serial))
}

/// The device path to open the volume at `mount`: `\\.\C:` for a drive
/// root, `\\?\Volume{GUID}` for a volume mounted in a folder.
pub(super) fn device_path(mount: &str) -> io::Result<String> {
    let plain = mount.strip_prefix(r"\\?\").unwrap_or(mount);
    let bytes = plain.as_bytes();
    if bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && &bytes[1..] == b":\\" {
        return Ok(format!(r"\\.\{}", &plain[..2]));
    }
    let input = wide(OsStr::new(mount));
    let mut output = vec![0u16; NAME_CAPACITY];
    let ok = unsafe {
        GetVolumeNameForVolumeMountPointW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32)
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let name = until_nul(&output);
    match name.strip_suffix('\\') {
        Some(device) if device.starts_with(r"\\?\Volume{") => Ok(device.to_string()),
        _ => Err(io::Error::other("unerwarteter Volume-Name")),
    }
}

/// Attributes, 32-bit volume serial and 64-bit file reference of an open handle.
pub(super) fn identity(file: &File) -> io::Result<(u32, u32, u64)> {
    // SAFETY: plain-data struct the call fills completely.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let reference = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok((info.dwFileAttributes, info.dwVolumeSerialNumber, reference))
}

fn ioctl(file: &File, code: u32, input: Option<&i64>, output: &mut [u8]) -> io::Result<usize> {
    let (input, input_len) = match input {
        Some(value) => (
            (value as *const i64).cast::<c_void>(),
            size_of::<i64>() as u32,
        ),
        None => (null(), 0),
    };
    let output_len = u32::try_from(output.len()).map_err(io::Error::other)?;
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            code,
            input,
            input_len,
            output.as_mut_ptr().cast::<c_void>(),
            output_len,
            &mut returned,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(returned as usize)
}

fn volume_data(volume: &File) -> io::Result<NtfsVolumeData> {
    let mut storage = Vec::new();
    let buffer = aligned(&mut storage, 8, VOLUME_DATA_LEN)?;
    let returned = ioctl(volume, FSCTL_GET_NTFS_VOLUME_DATA, None, buffer)?;
    parse_volume_data(buffer.get(..returned).unwrap_or(&[]))
}

/// The extents of `$MFT::$DATA`, queried in batches until complete.
fn mft_extents(device: &str) -> io::Result<Vec<MftExtent>> {
    let mft = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(format!(r"{device}\$MFT::$DATA"))?;
    let mut storage = Vec::new();
    let buffer = aligned(&mut storage, 8, 64 * 1024)?;
    let mut extents = Vec::new();
    let mut start = 0i64;
    loop {
        match ioctl(&mft, FSCTL_GET_RETRIEVAL_POINTERS, Some(&start), buffer) {
            Ok(returned) => {
                let (batch, _) = parse_retrieval_pointers(buffer.get(..returned).unwrap_or(&[]))?;
                extents.extend(batch);
                return Ok(extents);
            }
            Err(error) if error.raw_os_error() == Some(ERROR_MORE_DATA as i32) => {
                let (batch, next) = parse_retrieval_pointers(buffer)?;
                if batch.is_empty() {
                    return Err(error);
                }
                extents.extend(batch);
                start = i64::try_from(next).map_err(io::Error::other)?;
            }
            Err(error) => return Err(error),
        }
    }
}

fn read_exact_at(volume: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    let mut done = 0usize;
    while done < buffer.len() {
        let read = volume.seek_read(&mut buffer[done..], offset + done as u64)?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        done += read;
    }
    Ok(())
}

/// Write the volume's cached metadata (MFT records of recently changed
/// files included) to disk, so the unbuffered reads below see it.
/// Documented way: `FlushFileBuffers` on a volume handle with
/// `GENERIC_WRITE` flushes all open files of the volume and needs
/// administrative rights (learn.microsoft.com, "FlushFileBuffers function",
/// Remarks; checked 2026-11-18). `File::sync_all` is that call on Windows.
fn flush_volume(device: &str) -> io::Result<()> {
    OpenOptions::new()
        .access_mode(GENERIC_READ_WRITE)
        .share_mode(SHARE_ALL)
        .open(device)?
        .sync_all()
}

/// Read the whole valid `$MFT` of the volume at `device` into an index.
/// `serial` (low 32 bits) must match, so the handle is the checked volume.
pub(super) fn read_index(
    device: &str,
    serial: u32,
    control: &MftControl<'_>,
) -> io::Result<MftIndex> {
    // A failed flush does not stop the scan; the index says what it may miss.
    let flushed = flush_volume(device);
    let volume = OpenOptions::new()
        .access_mode(FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_NO_BUFFERING)
        .open(device)?;
    let data = volume_data(&volume)?;
    if data.serial as u32 != serial {
        return Err(io::Error::other(
            "Volume hat sich zwischen Prüfung und Lesen geändert",
        ));
    }
    let spans = plan_reads(&data, &mft_extents(device)?, CHUNK)?;
    let total: u64 = spans.iter().map(|span| span.len).sum();
    let mut reader = MftRecordReader::new(&data)?;
    if let Err(error) = &flushed {
        reader.note_unflushed(error);
    }
    let align = (data.bytes_per_sector as usize).max(4096);
    let mut storage = Vec::new();
    let mut done = 0u64;
    for span in spans {
        if control.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
        }
        match span.disk {
            None => reader.skip(span.logical, span.len)?,
            Some(offset) => {
                let buffer = aligned(&mut storage, align, span.len as usize)?;
                read_exact_at(&volume, buffer, offset)?;
                reader.feed(span.logical, buffer)?;
            }
        }
        done += span.len;
        (control.progress)(done, total);
    }
    Ok(reader.finish())
}
