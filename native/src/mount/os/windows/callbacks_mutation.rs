use std::io;

use windows_sys::Win32::Foundation::ERROR_INVALID_HANDLE;

use crate::mount::{FlushOutcome, MountStatus, RenameOutcome};

use super::{
    callback_context::{context_key, CallbackContext, HandleSnapshot, NodeHandle},
    callback_status::{guard_long_with_context, void_guard_long, win32},
    wide::read_wide,
    DokanFileInfo, NtStatus,
};

pub(super) unsafe extern "system" fn delete_file(
    file_name: *const u16,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe { begin_delete(file_name, false, file_info) }
}

pub(super) unsafe extern "system" fn delete_directory(
    file_name: *const u16,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe { begin_delete(file_name, true, file_info) }
}

unsafe fn begin_delete(
    file_name: *const u16,
    is_directory: bool,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_long_with_context(file_info, |context| {
            let path = read_wide(file_name)?;
            let key = context_key(file_info)?;
            let snapshot = context.snapshot(key)?;
            if !context.path_matches(&snapshot.path, &path)
                || matches!(
                    (snapshot.node, is_directory),
                    (NodeHandle::File(_), true) | (NodeHandle::Directory, false)
                )
            {
                return Err(win32(ERROR_INVALID_HANDLE));
            }
            let delete_requested = file_info
                .as_ref()
                .is_some_and(|info| info.delete_pending != 0);
            if delete_requested {
                context.request_delete(key, &path, is_directory)?;
            } else {
                // Dokany sends a second callback with DeletePending cleared
                // when Windows cancels a previously requested disposition.
                context.cancel_delete(key, &path, is_directory)?;
            }
            Ok(())
        })
    }
}

pub(super) unsafe extern "system" fn move_file(
    file_name: *const u16,
    new_file_name: *const u16,
    replace_existing: i32,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe {
        guard_long_with_context(file_info, |context| {
            let source = read_wide(file_name)?;
            let destination = read_wide(new_file_name)?;
            let key = context_key(file_info)?;
            if !context.path_matches(&context.snapshot(key)?.path, &source) {
                return Err(win32(ERROR_INVALID_HANDLE));
            }
            let replace_existing = replace_existing != 0;
            let rename = context.reserve_rename(key, &source, &destination, replace_existing)?;
            let outcome = context.engine.rename_with_shared_destination(
                &source,
                &destination,
                replace_existing,
                rename.destination_is_open(),
            )?;
            if let Err(error) = rename.commit() {
                context.report(MountStatus::Failed {
                    detail: "open handle state could not follow a completed remote rename".into(),
                });
                context.request_stop();
                let _ = error;
                // The remote namespace mutation already succeeded. Returning
                // failure would invite a replay; stop and remount instead.
                return Ok(());
            }
            if let RenameOutcome::CommittedPendingVerification(conflict) = outcome {
                let drive = context.selected_drive()?;
                context.report(MountStatus::Conflict {
                    drive,
                    path: conflict.path,
                    detail: conflict.detail,
                });
            }
            Ok(())
        })
    }
}

pub(super) unsafe extern "system" fn cleanup(
    _file_name: *const u16,
    file_info: *mut DokanFileInfo,
) {
    unsafe {
        void_guard_long(file_info, |context| {
            let key = match context_key(file_info) {
                Ok(key) => key,
                Err(_) => return Ok(()),
            };
            // IRP_MJ_CLEANUP releases the Windows share reservation even when
            // mapped paging I/O keeps the underlying context alive until Close.
            let snapshot = context.cleanup_handle(key)?;
            if snapshot.delete_requested {
                context.commit_delete(key, &snapshot.path, node_is_directory(snapshot.node))?;
                return Ok(());
            }
            if snapshot.delete_committed {
                return Ok(());
            }
            flush_for_cleanup(context, &snapshot)
        });
    }
}

pub(super) unsafe extern "system" fn close_file(
    _file_name: *const u16,
    file_info: *mut DokanFileInfo,
) {
    unsafe {
        void_guard_long(file_info, |context| {
            let key = match context_key(file_info) {
                Ok(key) => key,
                Err(_) => return Ok(()),
            };
            // Close is the final safety net if Dokany omitted Cleanup or its
            // DeletePending flag diverged from the successfully prepared
            // internal transaction. Release sharing first, then finalize from
            // the authoritative per-handle state before removing the record.
            let cleanup_snapshot = match context.cleanup_handle(key) {
                Ok(snapshot) => snapshot,
                Err(error) => return Err(error),
            };
            let mut failure = None;
            if cleanup_snapshot.delete_requested {
                if let Err(error) = context.commit_delete(
                    key,
                    &cleanup_snapshot.path,
                    node_is_directory(cleanup_snapshot.node),
                ) {
                    failure = Some(error);
                }
            } else if !cleanup_snapshot.delete_committed {
                if let Err(error) = flush_for_cleanup(context, &cleanup_snapshot) {
                    failure = Some(error);
                }
            }
            let snapshot = match context.take(key) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    if let Some(info) = file_info.as_mut() {
                        info.context = 0;
                    }
                    return Err(error);
                }
            };
            if let Some(info) = file_info.as_mut() {
                info.context = 0;
            }
            if let NodeHandle::File(handle) = snapshot.node {
                if let Err(error) = context.engine.close(handle) {
                    failure.get_or_insert(error);
                }
            }
            failure.map_or(Ok(()), Err)
        });
    }
}

fn node_is_directory(node: NodeHandle) -> bool {
    matches!(node, NodeHandle::Directory)
}

fn flush_for_cleanup(context: &CallbackContext, snapshot: &HandleSnapshot) -> io::Result<()> {
    let NodeHandle::File(handle) = snapshot.node else {
        return Ok(());
    };
    match context.engine.flush(handle)? {
        FlushOutcome::NoChanges | FlushOutcome::Committed => Ok(()),
        FlushOutcome::CommittedPendingVerification(conflict) | FlushOutcome::Conflict(conflict) => {
            let drive = context.selected_drive()?;
            context.report(MountStatus::Conflict {
                drive,
                path: conflict.path,
                detail: conflict.detail,
            });
            Ok(())
        }
    }
}
