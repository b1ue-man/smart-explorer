//! Optional filesystem block clone, exclusively between the selected file handles.
use std::{ffi::c_void, fs::File, io, os::windows::io::AsRawHandle};
use windows_sys::Win32::System::IO::DeviceIoControl;

#[repr(C)]
struct Extents { source: *mut c_void, source_offset: i64, destination_offset: i64, length: i64 }

pub(super) fn clone(source: &File, destination: &File, size: u64) -> io::Result<bool> {
    let Ok(length) = i64::try_from(size) else { return Ok(false); };
    if size == 0 { return Ok(false); }
    let input_file = source.try_clone()?;
    let output_file = destination.try_clone()?;
    let input = Box::new(Extents { source: input_file.as_raw_handle(), source_offset: 0, destination_offset: 0, length });
    let mut returned = Box::new(0u32);
    destination.set_len(size)?;
    // FSCTL_DUPLICATE_EXTENTS_TO_FILE; unsupported filesystems and unaligned
    // tails use the ordinary cancellable stream without reopening either path.
    let success = unsafe { DeviceIoControl(output_file.as_raw_handle(), 0x0009_8344,
        (input.as_ref() as *const Extents).cast(), std::mem::size_of::<Extents>() as u32,
        std::ptr::null_mut(), 0, returned.as_mut(), std::ptr::null_mut()) };
    if success != 0 { return Ok(true); }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(997) {
        // Synchronous handles cannot normally return IO_PENDING. Preserve the
        // exact input/output buffers and handles if a driver violates that rule.
        std::mem::forget((input_file, output_file, input, returned));
        return Err(io::Error::other("block clone completion unconfirmed"));
    }
    // A clone-specific privilege may be unavailable even though both ordinary
    // data handles were granted. Stream I/O still enforces their real rights.
    if matches!(error.raw_os_error(), Some(1 | 5 | 17 | 50 | 87 | 1314)) { return Ok(false); }
    Err(error)
}
