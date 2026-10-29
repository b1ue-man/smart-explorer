//! What a small Windows source carries beyond bytes, attributes and times
//! (A7, 2026-10-29): asked of its open handle, never of its path. A source
//! for which any answer is missing or not plain keeps `CopyFile2`.
use std::ffi::c_void;
use std::fs::File;
use std::os::windows::io::AsRawHandle;

use windows_sys::Wdk::Storage::FileSystem::{
    FileEaInformation, FileFsDeviceInformation, NtQueryInformationFile,
    NtQueryVolumeInformationFile,
};
use windows_sys::Win32::Foundation::{LocalFree, STATUS_PENDING};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{ACL, ATTRIBUTE_SECURITY_INFORMATION};
use windows_sys::Win32::Storage::FileSystem::{FileStreamInfo, GetFileInformationByHandleEx};

use super::pending_block;

/// FILE_REMOTE_DEVICE in the volume's device characteristics.
const REMOTE_DEVICE: u32 = 0x10;

/// On a local volume, nothing but the unnamed data stream, no extended
/// attributes and no security resource attributes: what `CopyFile2` would
/// carry beyond bytes, attributes and times is absent. Any question that
/// cannot be answered counts as "not plain".
pub(super) fn plain_local_source(reader: &File) -> bool {
    !remote_volume(reader).unwrap_or(true)
        && only_data_stream(reader)
        && ea_size(reader) == Some(0)
        && resource_attributes(reader) == Some(false)
}

/// Whether the volume of `file` is a network volume (a mapped drive keeps
/// `CopyFile2`, which copies on the server where it can).
pub(super) fn remote_volume(file: &File) -> Option<bool> {
    let mut device = Box::new([0u32; 2]);
    let mut completion = Box::new(pending_block());
    // SAFETY: a live synchronous handle; both outputs live in boxes that are
    // leaked below if the call were ever still pending.
    let status = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            completion.as_mut(),
            device.as_mut_ptr().cast(),
            8,
            FileFsDeviceInformation,
        )
    };
    if status == STATUS_PENDING {
        std::mem::forget((device, completion));
        return None;
    }
    (status >= 0 && completion.Information == 8).then(|| device[1] & REMOTE_DEVICE != 0)
}

/// Exactly one stream, the unnamed data stream `::$DATA`.
fn only_data_stream(file: &File) -> bool {
    // FILE_STREAM_INFO: next offset, name length, size, allocation, name.
    let mut buffer = [0u64; 16];
    // SAFETY: a live handle and an aligned buffer of the stated size.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStreamInfo,
            buffer.as_mut_ptr().cast(),
            std::mem::size_of_val(&buffer) as u32,
        )
    } == 0
    {
        return false;
    }
    let words = buffer.as_ptr().cast::<u32>();
    // SAFETY: the record starts the buffer; its two leading u32 fields and
    // the name at byte 24 lie inside it (checked against the buffer size).
    let (next, name_bytes) = unsafe { (*words, *words.add(1) as usize) };
    let data: Vec<u16> = "::$DATA".encode_utf16().collect();
    if next != 0 || name_bytes != data.len() * 2 || 24 + name_bytes > std::mem::size_of_val(&buffer)
    {
        return false;
    }
    // SAFETY: see above; the name is inside the buffer.
    let name = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<u8>().add(24).cast::<u16>(),
            data.len(),
        )
    };
    name == data.as_slice()
}

/// The size of the file's extended attributes (FILE_EA_INFORMATION).
fn ea_size(file: &File) -> Option<u32> {
    let mut size = Box::new(0u32);
    let mut completion = Box::new(pending_block());
    // SAFETY: a live synchronous handle; outputs boxed as in `remote_volume`.
    let status = unsafe {
        NtQueryInformationFile(
            file.as_raw_handle(),
            completion.as_mut(),
            (size.as_mut() as *mut u32).cast(),
            4,
            FileEaInformation,
        )
    };
    if status == STATUS_PENDING {
        std::mem::forget((size, completion));
        return None;
    }
    (status >= 0).then_some(*size)
}

/// Whether the file carries security resource attributes (which
/// `CopyFile2` copies and a plain create does not); needs READ_CONTROL,
/// which the source's read access includes.
fn resource_attributes(file: &File) -> Option<bool> {
    let mut list: *mut ACL = std::ptr::null_mut();
    let mut descriptor: *mut c_void = std::ptr::null_mut();
    // SAFETY: a live handle; the outputs are written by the call and the
    // descriptor is freed once below.
    let code = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            ATTRIBUTE_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut list,
            &mut descriptor,
        )
    };
    // SAFETY: `list` points into `descriptor`, read before it is freed.
    let present = (code == 0).then(|| !list.is_null() && unsafe { (*list).AceCount } > 0);
    if !descriptor.is_null() {
        // SAFETY: allocated by GetSecurityInfo for this caller.
        unsafe { LocalFree(descriptor) };
    }
    present
}
