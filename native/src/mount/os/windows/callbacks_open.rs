use std::io;

use windows_sys::Win32::Foundation::{
    ERROR_DIRECTORY, ERROR_NOT_SUPPORTED, STATUS_OBJECT_NAME_COLLISION,
};

use crate::mount::{MountStatus, NamespaceOutcome, OpenDisposition, OpenFileOptions};

use super::{
    callback_context::{set_context_key, CallbackContext, NodeHandle},
    callback_status::{guard_long_with_context, win32, CallbackFailure, CallbackResult},
    metadata::reject_open_symlink,
    wide::read_wide,
    DokanFileInfo, DokanIoSecurityContext, NtStatus,
};

const FILE_SUPERSEDE: u32 = 0;
const FILE_OPEN: u32 = 1;
const FILE_CREATE: u32 = 2;
const FILE_OPEN_IF: u32 = 3;
const FILE_OVERWRITE: u32 = 4;
const FILE_OVERWRITE_IF: u32 = 5;
const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
const FILE_DELETE_ON_CLOSE: u32 = 0x0000_1000;
const FILE_OPEN_BY_FILE_ID: u32 = 0x0000_2000;
const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_WRITE_DATA: u32 = 0x0000_0002;
const FILE_APPEND_DATA: u32 = 0x0000_0004;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

pub(super) unsafe extern "system" fn create_file(
    file_name: *const u16,
    _security_context: *mut DokanIoSecurityContext,
    desired_access: u32,
    file_attributes: u32,
    share_access: u32,
    create_disposition: u32,
    create_options: u32,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_long_with_context(file_info, |context| {
            let path = read_wide(file_name)?;
            validate_create_flags(file_attributes, create_options)?;
            let existing = match context.engine.stat(&path) {
                Ok(meta) => Some(meta),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            if let Some(meta) = &existing {
                reject_open_symlink(meta)?;
            }
            let explicitly_directory = create_options & FILE_DIRECTORY_FILE != 0;
            let explicitly_file = create_options & FILE_NON_DIRECTORY_FILE != 0;
            let is_directory = explicitly_directory
                || (!explicitly_file && existing.as_ref().is_some_and(|meta| meta.is_dir));
            if file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && !is_directory {
                return Err(win32(ERROR_DIRECTORY));
            }
            if is_directory {
                open_directory(
                    context,
                    path,
                    existing.as_ref().map(|meta| meta.is_dir),
                    desired_access,
                    create_disposition,
                    create_options,
                    share_access,
                    file_info,
                )
            } else {
                if existing.as_ref().is_some_and(|meta| meta.is_dir) {
                    return Err(win32(ERROR_DIRECTORY));
                }
                open_regular_file(
                    context,
                    path,
                    existing.is_some(),
                    desired_access,
                    create_disposition,
                    create_options,
                    share_access,
                    file_info,
                )
            }
        })
    }
}

fn open_directory(
    context: &CallbackContext,
    path: String,
    existing_is_directory: Option<bool>,
    desired_access: u32,
    disposition: u32,
    create_options: u32,
    share_access: u32,
    file_info: *mut DokanFileInfo,
) -> CallbackResult {
    let reservation = context.reserve_handle(&path, true, desired_access, share_access)?;
    match (existing_is_directory, disposition) {
        (Some(false), _) => return Err(win32(ERROR_DIRECTORY)),
        (Some(true), FILE_CREATE) => {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "directory exists").into())
        }
        (Some(true), FILE_OPEN | FILE_OPEN_IF) => {}
        (Some(true), _) => return Err(win32(ERROR_NOT_SUPPORTED)),
        (None, FILE_CREATE | FILE_OPEN_IF) => match context.engine.mkdir(&path)? {
            NamespaceOutcome::Complete => {}
            NamespaceOutcome::CommittedPendingVerification { path, detail } => {
                context.report(MountStatus::Failed {
                    detail: format!("{detail} ({path})"),
                });
                context.request_stop();
            }
        },
        (None, FILE_OPEN) => {
            return Err(io::Error::new(io::ErrorKind::NotFound, "directory not found").into())
        }
        (None, _) => return Err(win32(ERROR_NOT_SUPPORTED)),
    }
    let delete_on_close = create_options & FILE_DELETE_ON_CLOSE != 0;
    if let Err(error) = reservation.bind(NodeHandle::Directory) {
        return Err(error.into());
    }
    let key = reservation.key();
    if let Err(error) = unsafe { set_context_key(file_info, key, true) } {
        return Err(error.into());
    }
    if delete_on_close {
        match reservation.request_delete_and_commit(&context.engine, true) {
            Ok(_) => {}
            Err(error) => {
                clear_context(file_info);
                return Err(error.into());
            }
        }
    } else {
        reservation.commit();
    }
    opened_existing(existing_is_directory.is_some(), disposition)
}

fn open_regular_file(
    context: &CallbackContext,
    path: String,
    exists: bool,
    desired_access: u32,
    disposition: u32,
    create_options: u32,
    share_access: u32,
    file_info: *mut DokanFileInfo,
) -> CallbackResult {
    let writable = desired_access
        & (FILE_WRITE_DATA | FILE_APPEND_DATA | MAXIMUM_ALLOWED | GENERIC_WRITE | GENERIC_ALL)
        != 0;
    let requested_disposition = disposition;
    let disposition = disposition_for_file(requested_disposition, exists, writable)?;
    let reservation = context.reserve_handle(&path, false, desired_access, share_access)?;
    let engine_handle = context.engine.open_file(
        &path,
        OpenFileOptions {
            writable,
            disposition,
        },
    )?;
    let delete_on_close = create_options & FILE_DELETE_ON_CLOSE != 0;
    if let Err(error) = reservation.bind(NodeHandle::File(engine_handle)) {
        let _ = context.engine.close(engine_handle);
        return Err(error.into());
    }
    let key = reservation.key();
    if let Err(error) = unsafe { set_context_key(file_info, key, false) } {
        let _ = context.engine.close(engine_handle);
        return Err(error.into());
    }
    if delete_on_close {
        match reservation.request_delete_and_commit(&context.engine, false) {
            Ok(_) => {}
            Err(error) => {
                clear_context(file_info);
                let _ = context.engine.close(engine_handle);
                return Err(error.into());
            }
        }
    } else {
        reservation.commit();
    }
    opened_existing(exists, requested_disposition)
}

fn clear_context(file_info: *mut DokanFileInfo) {
    unsafe {
        if let Some(info) = file_info.as_mut() {
            info.context = 0;
        }
    }
}

/// Dokany uses STATUS_OBJECT_NAME_COLLISION as the successful "opened rather
/// than created" signal for the kernel's SUPERSEDE/OPEN_IF/OVERWRITE_IF
/// dispositions (the Win32 CREATE_ALWAYS and OPEN_ALWAYS families).
/// The callback context must already be installed before returning it.
fn opened_existing(exists: bool, disposition: u32) -> CallbackResult {
    if exists
        && matches!(
            disposition,
            FILE_SUPERSEDE | FILE_OPEN_IF | FILE_OVERWRITE_IF
        )
    {
        Err(CallbackFailure::Nt(STATUS_OBJECT_NAME_COLLISION))
    } else {
        Ok(())
    }
}

fn disposition_for_file(
    value: u32,
    exists: bool,
    writable: bool,
) -> Result<OpenDisposition, CallbackFailure> {
    match value {
        FILE_OPEN => Ok(OpenDisposition::OpenExisting),
        FILE_OPEN_IF if exists && !writable => Ok(OpenDisposition::OpenExisting),
        FILE_OPEN_IF => Ok(OpenDisposition::OpenOrCreate),
        FILE_CREATE => Ok(OpenDisposition::CreateNew),
        FILE_OVERWRITE => Ok(OpenDisposition::TruncateExisting),
        FILE_SUPERSEDE | FILE_OVERWRITE_IF => Ok(OpenDisposition::CreateAlways),
        _ => Err(win32(ERROR_NOT_SUPPORTED)),
    }
}

fn validate_create_flags(attributes: u32, options: u32) -> Result<(), CallbackFailure> {
    if options & (FILE_OPEN_BY_FILE_ID | FILE_OPEN_REPARSE_POINT) != 0 {
        return Err(win32(ERROR_NOT_SUPPORTED));
    }
    if attributes & !(FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_ARCHIVE | FILE_ATTRIBUTE_NORMAL)
        != 0
    {
        return Err(win32(ERROR_NOT_SUPPORTED));
    }
    if options & FILE_DIRECTORY_FILE != 0 && options & FILE_NON_DIRECTORY_FILE != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "file cannot be both a directory and a regular file",
        )
        .into());
    }
    Ok(())
}
