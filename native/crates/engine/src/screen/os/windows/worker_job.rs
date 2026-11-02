//! Lifetime of the screen host process on Windows (SCR-M3): every host
//! process joins one job object of the daemon that kills its processes when
//! the last handle to it closes (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`). The
//! daemon keeps that handle until it exits, so its end, even a crash, ends
//! every host process; the end of their input is the second sign. Host
//! processes start without a console window (`CREATE_NO_WINDOW`), because
//! `se.exe` is a console program and the daemon may run without one.
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::sync::{Mutex, PoisonError};

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

/// The job of this daemon. Its handle stays open for the life of the
/// process on purpose: closing it would kill every host process.
struct Job(HANDLE);

// SAFETY: a kernel handle may be used from every thread; it is only read.
unsafe impl Send for Job {}

static JOB: Mutex<Option<Job>> = Mutex::new(None);

fn create_job() -> io::Result<Job> {
    // SAFETY: null attributes and name create an unnamed job with default
    // security; the result is checked below.
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: an all-zero JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a valid
    // value (no limits); only the flag below is set.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `limits` lives across the call and the size matches its type.
    let changed = unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if changed == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Job(handle))
}

/// Before the start: no console window.
pub(super) fn prepare(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

/// After the start: the process joins the job of this daemon.
pub(super) fn bind(child: &Child) -> io::Result<()> {
    let mut slot = JOB.lock().unwrap_or_else(PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(create_job()?);
    }
    let Some(job) = slot.as_ref() else {
        return Err(io::Error::other("screen host job is missing"));
    };
    // SAFETY: both handles are valid: the job lives in `JOB`, the process
    // handle in `child`, which the caller still owns.
    let assigned = unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) };
    if assigned == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// In the host process: the job of the daemon does the work.
pub(super) fn arm_worker() -> io::Result<()> {
    Ok(())
}
