use std::{
    cell::Cell,
    io,
    panic::AssertUnwindSafe,
    time::{Duration, Instant},
};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_DIRECTORY, ERROR_DIR_NOT_EMPTY,
    ERROR_DISK_FULL, ERROR_FILE_NOT_FOUND, ERROR_GEN_FAILURE, ERROR_HANDLE_EOF,
    ERROR_HOST_UNREACHABLE, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER,
    ERROR_NETWORK_UNREACHABLE, ERROR_NOT_ENOUGH_MEMORY, ERROR_NOT_SAME_DEVICE, ERROR_NOT_SUPPORTED,
    ERROR_OPERATION_ABORTED, ERROR_SEM_TIMEOUT, ERROR_SHARING_VIOLATION, NO_ERROR,
    STATUS_BUFFER_OVERFLOW, STATUS_INVALID_PARAMETER, STATUS_IO_DEVICE_ERROR, STATUS_SUCCESS,
    STATUS_UNHANDLED_EXCEPTION,
};

use crate::mount::MountStatus;

use super::{
    callback_context::{context_from_file_info, CallbackContext},
    DokanFileInfo, NtStatus,
};

#[path = "dokany_status_table.rs"]
pub(super) mod dokany_status_table;
use dokany_status_table::dokany_maps_win32;

pub(super) const CALLBACK_TIMEOUT_MS: u32 = 300_000;
const SLOW_CALLBACK: Duration = Duration::from_millis(500);

pub(super) enum CallbackFailure {
    Io(io::Error),
    Win32(u32),
    Nt(NtStatus),
}

impl From<io::Error> for CallbackFailure {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(super) type CallbackResult = Result<(), CallbackFailure>;

pub(super) unsafe fn guard_with_context<F>(file_info: *mut DokanFileInfo, operation: F) -> NtStatus
where
    F: FnOnce(&CallbackContext) -> CallbackResult,
{
    unsafe { guard_caught(file_info, false, operation) }
}

pub(super) unsafe fn guard_long_with_context<F>(
    file_info: *mut DokanFileInfo,
    operation: F,
) -> NtStatus
where
    F: FnOnce(&CallbackContext) -> CallbackResult,
{
    unsafe { guard_caught(file_info, true, operation) }
}

unsafe fn guard_caught<F>(
    file_info: *mut DokanFileInfo,
    keep_timeout_alive: bool,
    operation: F,
) -> NtStatus
where
    F: FnOnce(&CallbackContext) -> CallbackResult,
{
    let acknowledged = Cell::new(None);
    match std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        guard_impl(file_info, keep_timeout_alive, operation, &acknowledged)
    })) {
        Ok(status) => status,
        Err(_) => {
            unsafe { stop_after_boundary_panic(file_info) };
            // A teardown panic must not turn an already acknowledged remote
            // commit into a replayable Windows error.
            acknowledged.get().unwrap_or(STATUS_UNHANDLED_EXCEPTION)
        }
    }
}

unsafe fn guard_impl<F>(
    file_info: *mut DokanFileInfo,
    keep_timeout_alive: bool,
    operation: F,
    acknowledged: &Cell<Option<NtStatus>>,
) -> NtStatus
where
    F: FnOnce(&CallbackContext) -> CallbackResult,
{
    let started = Instant::now();
    let context = match std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        context_from_file_info(file_info)
    })) {
        Ok(Ok(context)) => context,
        Ok(Err(_)) => return STATUS_INVALID_PARAMETER,
        Err(_) => return STATUS_UNHANDLED_EXCEPTION,
    };
    let keepalive = if keep_timeout_alive {
        match context.supervise_timeout(file_info) {
            Ok(keepalive) => Some(keepalive),
            Err(error) => {
                context.report(MountStatus::Failed {
                    detail: format!("mounted-drive callback supervision is unavailable: {error}"),
                });
                context.request_stop();
                return io_status(context, &error);
            }
        }
    } else {
        None
    };
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| operation(context)));
    let status = match result {
        Ok(Ok(())) => STATUS_SUCCESS,
        Ok(Err(CallbackFailure::Io(error))) => io_status(context, &error),
        Ok(Err(CallbackFailure::Win32(error))) => {
            dokany_status(error, |code| context.runtime.nt_status_from_win32(code))
        }
        Ok(Err(CallbackFailure::Nt(status))) => status,
        Err(_) => {
            context.report(MountStatus::Failed {
                detail: "the mounted-drive callback stopped after an internal failure".into(),
            });
            context.request_stop();
            STATUS_UNHANDLED_EXCEPTION
        }
    };
    acknowledged.set(Some(status));
    if keepalive.is_some_and(|keepalive| !keepalive.finish()) {
        context.report(MountStatus::Failed {
            detail: "Dokany could not extend a running remote filesystem request".into(),
        });
        context.request_stop();
    }
    let elapsed = started.elapsed();
    if elapsed >= SLOW_CALLBACK {
        context.report_slow_callback(if keep_timeout_alive { "long" } else { "short" }, elapsed);
    }
    // Timeout supervision is diagnostic after the operation has completed. It
    // must never turn an acknowledged remote commit into a Windows failure
    // that invites the application to replay the save or rename.
    status
}

pub(super) unsafe fn void_guard_long<F>(file_info: *mut DokanFileInfo, operation: F)
where
    F: FnOnce(&CallbackContext) -> io::Result<()>,
{
    if std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        void_guard_long_impl(file_info, operation)
    }))
    .is_err()
    {
        unsafe { stop_after_boundary_panic(file_info) };
    }
}

unsafe fn void_guard_long_impl<F>(file_info: *mut DokanFileInfo, operation: F)
where
    F: FnOnce(&CallbackContext) -> io::Result<()>,
{
    let started = Instant::now();
    let context = match std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        context_from_file_info(file_info)
    })) {
        Ok(Ok(context)) => context,
        _ => return,
    };
    let keepalive = context.supervise_timeout(file_info);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| operation(context)));
    let keepalive_ok = keepalive.is_ok_and(|lease| lease.finish());
    if !keepalive_ok {
        context.report(MountStatus::Failed {
            detail: "Dokany could not supervise a running finalization request".into(),
        });
        context.request_stop();
    }
    match result {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            context.report(MountStatus::Failed {
                detail: "an open mounted-drive handle could not be finalized safely".into(),
            });
            context.request_stop();
        }
        Err(_) => {
            context.report(MountStatus::Failed {
                detail: "the mounted-drive callback stopped after an internal failure".into(),
            });
            context.request_stop();
        }
    }
    let elapsed = started.elapsed();
    if elapsed >= SLOW_CALLBACK {
        context.report_slow_callback("finalize", elapsed);
    }
}

unsafe fn stop_after_boundary_panic(file_info: *mut DokanFileInfo) {
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        if let Ok(context) = context_from_file_info(file_info) {
            context.request_stop();
        }
    }));
}

pub(super) fn win32(error: u32) -> CallbackFailure {
    CallbackFailure::Win32(error)
}

pub(super) fn unsupported() -> CallbackFailure {
    win32(ERROR_NOT_SUPPORTED)
}

pub(super) fn insufficient_buffer() -> CallbackFailure {
    CallbackFailure::Nt(STATUS_BUFFER_OVERFLOW)
}

fn io_status(context: &CallbackContext, error: &io::Error) -> NtStatus {
    nt_status_for_io(error, |code| context.runtime.nt_status_from_win32(code))
}

/// NTSTATUS of an engine error (decision C4). An OS error keeps its own
/// Win32 code, every other error gets the code of its kind; a code goes to
/// `DokanNtStatusFromWin32` only when Dokany has a row for it. Dokany turns
/// every other code into STATUS_ACCESS_DENIED, which Explorer reports as
/// "Access denied", so such errors, and kinds without a code, return
/// STATUS_IO_DEVICE_ERROR ("I/O device error") directly. Before C4 the
/// fallback ERROR_INVALID_DATA, which Dokany does not know, made every
/// unmapped failure look like missing rights.
pub(super) fn nt_status_for_io(error: &io::Error, translate: impl Fn(u32) -> NtStatus) -> NtStatus {
    let code = error
        .raw_os_error()
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| win32_for_kind(error.kind()));
    match code {
        Some(code) => dokany_status(code, translate),
        None => STATUS_IO_DEVICE_ERROR,
    }
}

/// A Win32 code through Dokany's table. An error never becomes success,
/// even with the code 0 (`NO_ERROR`, which Dokany maps to STATUS_SUCCESS).
fn dokany_status(code: u32, translate: impl Fn(u32) -> NtStatus) -> NtStatus {
    if code != NO_ERROR && dokany_maps_win32(code) {
        translate(code)
    } else {
        STATUS_IO_DEVICE_ERROR
    }
}

/// The Win32 code of an error kind, `None` for kinds no code describes.
/// Rows of v0.5.182 are unchanged except the three of C4: host and network
/// unreachable ("The network location cannot be reached.") and crossing
/// devices (moves between shares of the device tree).
pub(super) fn win32_for_kind(kind: io::ErrorKind) -> Option<u32> {
    let code = match kind {
        io::ErrorKind::NotFound => ERROR_FILE_NOT_FOUND,
        io::ErrorKind::NotADirectory | io::ErrorKind::IsADirectory => ERROR_DIRECTORY,
        io::ErrorKind::DirectoryNotEmpty => ERROR_DIR_NOT_EMPTY,
        io::ErrorKind::PermissionDenied => ERROR_ACCESS_DENIED,
        io::ErrorKind::AlreadyExists => ERROR_ALREADY_EXISTS,
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => ERROR_INVALID_PARAMETER,
        io::ErrorKind::TimedOut => ERROR_SEM_TIMEOUT,
        io::ErrorKind::WriteZero | io::ErrorKind::StorageFull => ERROR_DISK_FULL,
        io::ErrorKind::UnexpectedEof => ERROR_HANDLE_EOF,
        io::ErrorKind::Unsupported => ERROR_NOT_SUPPORTED,
        io::ErrorKind::OutOfMemory => ERROR_NOT_ENOUGH_MEMORY,
        io::ErrorKind::WouldBlock => ERROR_SHARING_VIOLATION,
        io::ErrorKind::Interrupted => ERROR_OPERATION_ABORTED,
        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionAborted => ERROR_INVALID_HANDLE,
        io::ErrorKind::HostUnreachable => ERROR_HOST_UNREACHABLE,
        io::ErrorKind::NetworkUnreachable => ERROR_NETWORK_UNREACHABLE,
        io::ErrorKind::CrossesDevices => ERROR_NOT_SAME_DEVICE,
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::NotConnected
        | io::ErrorKind::AddrInUse
        | io::ErrorKind::AddrNotAvailable
        | io::ErrorKind::NetworkDown => ERROR_GEN_FAILURE,
        _ => return None,
    };
    Some(code)
}
