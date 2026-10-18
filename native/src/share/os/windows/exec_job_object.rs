//! Kill-on-close Job Objects of Exec jobs. The worker's own account
//! (`exec.rs`) and the sessions of the account service
//! (`exec_account_spawn.rs`) contain their processes the same way: every
//! process of a Job ends when the last handle of the Job closes, and no
//! process may leave it (`JOB_OBJECT_LIMIT_BREAKAWAY_OK` is never set).
use std::io;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use super::exec_handles::OwnedHandle;

/// A new, empty Job that ends its processes when its last handle closes.
pub(super) fn create_job() -> io::Result<OwnedHandle> {
    let job = OwnedHandle::new(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
    let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let changed = unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&information as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if changed == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

/// How many processes of `job` still run.
pub(super) fn active_processes(job: HANDLE) -> io::Result<u32> {
    let mut information: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
    let queried = unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicAccountingInformation,
            (&mut information as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };
    if queried == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(information.ActiveProcesses)
    }
}

/// Ends every process of `job`; a Job that has no process left counts as
/// ended even when Windows refuses the call.
pub(super) fn terminate_job(job: HANDLE) -> io::Result<()> {
    if unsafe { TerminateJobObject(job, 1) } == 0 {
        let error = io::Error::last_os_error();
        if active_processes(job).unwrap_or(1) != 0 {
            return Err(error);
        }
    }
    Ok(())
}
