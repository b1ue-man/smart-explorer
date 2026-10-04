//! Native handle-relative rename; the caller keeps the DELETE reservation
//! and validates the destination's full identity after this I/O completes.
use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
use std::{ffi::OsStr, fs::File, io, mem::size_of};

use windows_sys::Wdk::Storage::FileSystem::{
    FileRenameInformation, NtSetInformationFile, FILE_RENAME_INFORMATION,
};
use windows_sys::Win32::Foundation::{
    RtlNtStatusToDosError, STATUS_PENDING, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{WaitForSingleObject, INFINITE};
use windows_sys::Win32::System::IO::{IO_STATUS_BLOCK, IO_STATUS_BLOCK_0};

use super::directory_handle::DirectoryHandle;

/// `name` is one child already checked by the quarantine boundary. `file` is
/// its synchronous, identity-confirmed DELETE guard; no free target path is used.
pub(super) fn no_replace(file: &File, target: &DirectoryHandle, name: &OsStr) -> io::Result<()> {
    let mut wide: Vec<u16> = name.encode_wide().collect();
    let name_bytes = wide
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename target too long"))?;
    wide.push(0); // Explicit terminator; FileNameLength excludes it.
                  // The native contract requires sizeof(record) plus the filename bytes.
                  // Keep a further terminator word rather than relying on structure padding.
    let bytes = size_of::<FILE_RENAME_INFORMATION>()
        .checked_add(name_bytes as usize)
        .and_then(|length| length.checked_add(size_of::<u16>()))
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "rename record too long"))?;
    let mut buffer = vec![0u64; (bytes as usize).div_ceil(size_of::<u64>())];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut completion = Box::new(IO_STATUS_BLOCK {
        Anonymous: IO_STATUS_BLOCK_0 {
            Status: STATUS_PENDING,
        },
        Information: 0,
    });
    // SAFETY: u64 storage is aligned for this native integer/handle record on
    // supported Windows targets and includes the complete name and terminator.
    // Both handles remain borrowed across the call and completion wait. Buffer
    // and IOSB storage survive even an unconfirmed wait (see `unconfirmed`).
    // Zero ReplaceIfExists refuses an existing destination; the actual pinned
    // directory handle binds this relative child to its destination object.
    let status = unsafe {
        (*info).Anonymous.ReplaceIfExists = 0;
        (*info).RootDirectory = target.file().as_raw_handle();
        (*info).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(
            wide.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            wide.len(),
        );
        NtSetInformationFile(
            file.as_raw_handle(),
            completion.as_mut(),
            info.cast(),
            bytes,
            FileRenameInformation,
        )
    };
    let status = if status == STATUS_PENDING {
        // CreateFile without OVERLAPPED supplies the synchronous guard. Keep
        // even an unexpected pending operation alive until its final IOSB.
        let waited = unsafe { WaitForSingleObject(file.as_raw_handle(), INFINITE) };
        if waited != WAIT_OBJECT_0 {
            let error = if waited == WAIT_FAILED {
                io::Error::last_os_error()
            } else {
                io::Error::other(format!("unexpected rename wait result {waited:#x}"))
            };
            return Err(unconfirmed(buffer, completion, target, name, error));
        }
        // SAFETY: a completed wait synchronizes the kernel's IOSB write.
        let final_status = unsafe { completion.Anonymous.Status };
        if final_status == STATUS_PENDING {
            return Err(unconfirmed(
                buffer,
                completion,
                target,
                name,
                io::Error::other("rename is still pending after completion wait"),
            ));
        }
        final_status
    } else {
        // Microsoft requires the direct status when the call is not pending.
        status
    };
    if status < 0 {
        // SAFETY: RtlNtStatusToDosError accepts this returned NTSTATUS value.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    Ok(())
}

fn unconfirmed(
    buffer: Vec<u64>,
    completion: Box<IO_STATUS_BLOCK>,
    target: &DirectoryHandle,
    name: &OsStr,
    error: io::Error,
) -> io::Error {
    // A failed wait cannot prove the kernel stopped using these addresses.
    // Retain only their memory until process exit; never release a live IOSB
    // or report a completed hop. The caller retains its durable recovery slots.
    std::mem::forget(buffer);
    std::mem::forget(completion);
    io::Error::new(
        error.kind(),
        format!(
            "rename completion unconfirmed at {}; recovery intent retained: {error}",
            target.path().join(name).display()
        ),
    )
}
