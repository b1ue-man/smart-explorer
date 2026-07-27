use std::{ffi::c_void, io, slice};

use windows_sys::Win32::Foundation::{
    ERROR_DIRECTORY, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER,
};

use crate::mount::{FlushOutcome, MountStatus};

use super::{
    callback_context::{context_key, NodeHandle},
    callback_status::{guard_long_with_context, win32},
    DokanFileInfo, NtStatus,
};

pub(super) unsafe extern "system" fn read_file(
    _file_name: *const u16,
    buffer: *mut c_void,
    buffer_length: u32,
    read_length: *mut u32,
    offset: i64,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        // Long guard: the first data access of a lazily opened handle fetches
        // the whole file, which needs timeout supervision.
        guard_long_with_context(file_info, |context| {
            initialize_count(read_length)?;
            let offset = nonnegative_offset(offset)?;
            let key = context_key(file_info)?;
            let snapshot = context.snapshot(key)?;
            let NodeHandle::File(handle) = snapshot.node else {
                return Err(win32(ERROR_DIRECTORY));
            };
            let output = output_buffer(buffer, buffer_length)?;
            let read = context.engine.read(handle, offset, output)?;
            *read_length = u32::try_from(read).map_err(|_| win32(ERROR_INVALID_PARAMETER))?;
            Ok(())
        })
    }
}

pub(super) unsafe extern "system" fn write_file(
    _file_name: *const u16,
    buffer: *const c_void,
    buffer_length: u32,
    written_length: *mut u32,
    offset: i64,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_long_with_context(file_info, |context| {
            initialize_count(written_length)?;
            let key = context_key(file_info)?;
            let snapshot = context.snapshot(key)?;
            let NodeHandle::File(handle) = snapshot.node else {
                return Err(win32(ERROR_DIRECTORY));
            };
            let input = input_buffer(buffer, buffer_length)?;
            let info = file_info
                .as_ref()
                .ok_or_else(|| win32(ERROR_INVALID_HANDLE))?;
            let append = info.write_to_end_of_file != 0 || offset == -1;
            if append && info.paging_io != 0 {
                // Paging writes may not extend a file. The previous EOF-based
                // path also produced a zero-byte write in this case.
                return Ok(());
            }
            if append {
                let written = context.engine.append(handle, input)?;
                *written_length =
                    u32::try_from(written).map_err(|_| win32(ERROR_INVALID_PARAMETER))?;
                return Ok(());
            }
            let write_offset = nonnegative_offset(offset)?;
            let mut allowed = input.len();
            if info.paging_io != 0 {
                let current_len = context.engine.len(handle)?;
                if write_offset >= current_len {
                    return Ok(());
                }
                allowed =
                    allowed.min(usize::try_from(current_len - write_offset).unwrap_or(usize::MAX));
            }
            let written = context
                .engine
                .write(handle, write_offset, &input[..allowed])?;
            *written_length = u32::try_from(written).map_err(|_| win32(ERROR_INVALID_PARAMETER))?;
            Ok(())
        })
    }
}

pub(super) unsafe extern "system" fn flush_file_buffers(
    _file_name: *const u16,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_long_with_context(file_info, |context| {
            let key = context_key(file_info)?;
            let snapshot = context.snapshot(key)?;
            let NodeHandle::File(handle) = snapshot.node else {
                return Ok(());
            };
            let outcome = match context.engine.flush(handle) {
                Ok(outcome) => outcome,
                Err(error) => {
                    context.report(MountStatus::Failed {
                        detail: "A mounted file could not be committed; its recovery cache remains available for Retry"
                            .into(),
                    });
                    return Err(error.into());
                }
            };
            match outcome {
                FlushOutcome::NoChanges | FlushOutcome::Committed => Ok(()),
                FlushOutcome::CommittedPendingVerification(conflict) => {
                    let drive = context.selected_drive()?;
                    context.report(MountStatus::Conflict {
                        drive,
                        path: conflict.path,
                        detail: conflict.detail,
                    });
                    // The backend already committed the atomic promotion.
                    // Returning an error would invite the application to retry
                    // a save whose remote namespace has already changed.
                    Ok(())
                }
                FlushOutcome::Conflict(conflict) => {
                    let drive = context.selected_drive()?;
                    context.report(MountStatus::Conflict {
                        drive,
                        path: conflict.path,
                        detail: conflict.detail,
                    });
                    Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "remote file changed while the mounted copy was open",
                    )
                    .into())
                }
            }
        })
    }
}

pub(super) unsafe extern "system" fn set_end_of_file(
    _file_name: *const u16,
    size: i64,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_long_with_context(file_info, |context| {
            let size = nonnegative_offset(size)?;
            let key = context_key(file_info)?;
            let snapshot = context.snapshot(key)?;
            let NodeHandle::File(handle) = snapshot.node else {
                return Err(win32(ERROR_DIRECTORY));
            };
            context.engine.truncate(handle, size)?;
            Ok(())
        })
    }
}

pub(super) unsafe extern "system" fn set_allocation_size(
    _file_name: *const u16,
    size: i64,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_long_with_context(file_info, |context| {
            let size = nonnegative_offset(size)?;
            let key = context_key(file_info)?;
            let snapshot = context.snapshot(key)?;
            let NodeHandle::File(handle) = snapshot.node else {
                return Err(win32(ERROR_DIRECTORY));
            };
            if size < context.engine.len(handle)? {
                context.engine.truncate(handle, size)?;
            }
            Ok(())
        })
    }
}

unsafe fn initialize_count(output: *mut u32) -> Result<(), io::Error> {
    let output = unsafe { output.as_mut() }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing byte count"))?;
    *output = 0;
    Ok(())
}

unsafe fn output_buffer<'a>(buffer: *mut c_void, length: u32) -> Result<&'a mut [u8], io::Error> {
    if length == 0 {
        return Ok(&mut []);
    }
    if buffer.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing read buffer",
        ));
    }
    Ok(unsafe { slice::from_raw_parts_mut(buffer.cast::<u8>(), length as usize) })
}

unsafe fn input_buffer<'a>(buffer: *const c_void, length: u32) -> Result<&'a [u8], io::Error> {
    if length == 0 {
        return Ok(&[]);
    }
    if buffer.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing write buffer",
        ));
    }
    Ok(unsafe { slice::from_raw_parts(buffer.cast::<u8>(), length as usize) })
}

fn nonnegative_offset(value: i64) -> Result<u64, super::callback_status::CallbackFailure> {
    u64::try_from(value).map_err(|_| win32(ERROR_INVALID_PARAMETER))
}
