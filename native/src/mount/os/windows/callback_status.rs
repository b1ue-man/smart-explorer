use std::{
    io,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_DIRECTORY, ERROR_DIR_NOT_EMPTY,
    ERROR_DISK_FULL, ERROR_FILE_NOT_FOUND, ERROR_GEN_FAILURE, ERROR_HANDLE_EOF, ERROR_INVALID_DATA,
    ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_NOT_SUPPORTED,
    ERROR_OPERATION_ABORTED, ERROR_SEM_TIMEOUT, ERROR_SHARING_VIOLATION, STATUS_BUFFER_OVERFLOW,
    STATUS_INVALID_PARAMETER, STATUS_SUCCESS, STATUS_UNHANDLED_EXCEPTION,
};

use crate::mount::MountStatus;

use super::{
    callback_context::{context_from_file_info, CallbackContext},
    DokanFileInfo, NtStatus,
};

pub(super) const CALLBACK_TIMEOUT_MS: u32 = 300_000;
const RESET_INTERVAL: Duration = Duration::from_secs(30);

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
    unsafe { guard_impl(file_info, false, operation) }
}

pub(super) unsafe fn guard_long_with_context<F>(
    file_info: *mut DokanFileInfo,
    operation: F,
) -> NtStatus
where
    F: FnOnce(&CallbackContext) -> CallbackResult,
{
    unsafe { guard_impl(file_info, true, operation) }
}

unsafe fn guard_impl<F>(
    file_info: *mut DokanFileInfo,
    keep_timeout_alive: bool,
    operation: F,
) -> NtStatus
where
    F: FnOnce(&CallbackContext) -> CallbackResult,
{
    let context = match std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        context_from_file_info(file_info)
    })) {
        Ok(Ok(context)) => context,
        Ok(Err(_)) => return STATUS_INVALID_PARAMETER,
        Err(_) => return STATUS_UNHANDLED_EXCEPTION,
    };
    let keepalive = if keep_timeout_alive {
        match TimeoutKeepalive::start(context, file_info) {
            Ok(keepalive) => Some(keepalive),
            Err(error) => return io_status(context, &error),
        }
    } else {
        None
    };
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| operation(context)));
    let status = match result {
        Ok(Ok(())) => STATUS_SUCCESS,
        Ok(Err(CallbackFailure::Io(error))) => io_status(context, &error),
        Ok(Err(CallbackFailure::Win32(error))) => context.runtime.nt_status_from_win32(error),
        Ok(Err(CallbackFailure::Nt(status))) => status,
        Err(_) => {
            context.report(MountStatus::Failed {
                detail: "the mounted-drive callback stopped after an internal failure".into(),
            });
            context.request_stop();
            STATUS_UNHANDLED_EXCEPTION
        }
    };
    if keepalive.is_some_and(|keepalive| !keepalive.finish()) {
        context.report(MountStatus::Failed {
            detail: "Dokany could not extend a running remote filesystem request".into(),
        });
        context.request_stop();
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
    let context = match std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        context_from_file_info(file_info)
    })) {
        Ok(Ok(context)) => context,
        _ => return,
    };
    let keepalive = TimeoutKeepalive::start(context, file_info);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| operation(context)));
    let keepalive_ok = keepalive.is_ok_and(TimeoutKeepalive::finish);
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
}

struct TimeoutKeepalive {
    state: Arc<TimeoutKeepaliveState>,
    thread: Option<JoinHandle<()>>,
}

struct TimeoutKeepaliveState {
    stop: Mutex<bool>,
    wake: Condvar,
    failed: AtomicBool,
}

impl TimeoutKeepalive {
    fn start(context: &CallbackContext, file_info: *mut DokanFileInfo) -> io::Result<Self> {
        if file_info.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing Dokany request for timeout keepalive",
            ));
        }
        let state = Arc::new(TimeoutKeepaliveState {
            stop: Mutex::new(false),
            wake: Condvar::new(),
            failed: AtomicBool::new(false),
        });
        let worker_state = Arc::clone(&state);
        let runtime = context.runtime.clone();
        let file_info_address = file_info as usize;
        let thread = std::thread::Builder::new()
            .name("mount-timeout-keepalive".into())
            .spawn(move || keepalive_loop(runtime, file_info_address, worker_state))?;
        Ok(Self {
            state,
            thread: Some(thread),
        })
    }

    fn finish(mut self) -> bool {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> bool {
        match self.state.stop.lock() {
            Ok(mut stop) => *stop = true,
            Err(poisoned) => *poisoned.into_inner() = true,
        }
        self.state.wake.notify_all();
        let joined = self
            .thread
            .take()
            .map(|thread| thread.join().is_ok())
            .unwrap_or(true);
        joined && !self.state.failed.load(Ordering::Acquire)
    }
}

impl Drop for TimeoutKeepalive {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

fn keepalive_loop(
    runtime: super::DokanyRuntime,
    file_info_address: usize,
    state: Arc<TimeoutKeepaliveState>,
) {
    let mut stop = match state.stop.lock() {
        Ok(stop) => stop,
        Err(_) => {
            state.failed.store(true, Ordering::Release);
            return;
        }
    };
    loop {
        if *stop {
            return;
        }
        let (next_stop, timeout) = match state.wake.wait_timeout(stop, RESET_INTERVAL) {
            Ok(waited) => waited,
            Err(_) => {
                state.failed.store(true, Ordering::Release);
                return;
            }
        };
        stop = next_stop;
        if *stop {
            return;
        }
        if !timeout.timed_out() {
            continue;
        }
        drop(stop);
        let reset = unsafe {
            runtime.reset_timeout(CALLBACK_TIMEOUT_MS, file_info_address as *mut DokanFileInfo)
        };
        if !reset {
            state.failed.store(true, Ordering::Release);
            return;
        }
        stop = match state.stop.lock() {
            Ok(stop) => stop,
            Err(_) => {
                state.failed.store(true, Ordering::Release);
                return;
            }
        };
    }
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
    if let Some(raw) = error
        .raw_os_error()
        .and_then(|value| u32::try_from(value).ok())
    {
        return context.runtime.nt_status_from_win32(raw);
    }
    let win32_error = match error.kind() {
        io::ErrorKind::NotFound => ERROR_FILE_NOT_FOUND,
        io::ErrorKind::NotADirectory | io::ErrorKind::IsADirectory => ERROR_DIRECTORY,
        io::ErrorKind::DirectoryNotEmpty => ERROR_DIR_NOT_EMPTY,
        io::ErrorKind::PermissionDenied => ERROR_ACCESS_DENIED,
        io::ErrorKind::AlreadyExists => ERROR_ALREADY_EXISTS,
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => ERROR_INVALID_PARAMETER,
        io::ErrorKind::TimedOut => ERROR_SEM_TIMEOUT,
        io::ErrorKind::WriteZero => ERROR_DISK_FULL,
        io::ErrorKind::UnexpectedEof => ERROR_HANDLE_EOF,
        io::ErrorKind::Unsupported => ERROR_NOT_SUPPORTED,
        io::ErrorKind::OutOfMemory => ERROR_NOT_ENOUGH_MEMORY,
        io::ErrorKind::WouldBlock => ERROR_SHARING_VIOLATION,
        io::ErrorKind::Interrupted => ERROR_OPERATION_ABORTED,
        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionAborted => ERROR_INVALID_HANDLE,
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::NotConnected
        | io::ErrorKind::AddrInUse
        | io::ErrorKind::AddrNotAvailable
        | io::ErrorKind::NetworkDown
        | io::ErrorKind::NetworkUnreachable
        | io::ErrorKind::HostUnreachable => ERROR_GEN_FAILURE,
        _ => ERROR_INVALID_DATA,
    };
    context.runtime.nt_status_from_win32(win32_error)
}
