use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, CREATE_BREAKAWAY_FROM_JOB, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
};

use super::super::OwnedHandle;
use super::{terminate_process_after_error, terminate_process_and_wait};

pub(super) struct SuspendedTestHost {
    pub(super) process: OwnedHandle,
    pub(super) thread: OwnedHandle,
}

pub(super) enum IsolationOutcome {
    Isolated(SuspendedTestHost),
    AmbientLocked,
}

pub(super) fn launch_suspended_test_host(mode: &str) -> io::Result<SuspendedTestHost> {
    launch_suspended_test_host_with_flags(mode, 0)
}

pub(super) fn launch_isolated_suspended_test_host(mode: &str) -> io::Result<IsolationOutcome> {
    let child = launch_suspended_test_host(mode)?;
    let inherited_job = match process_is_in_any_job(&child.process) {
        Ok(inherited_job) => inherited_job,
        Err(error) => {
            return Err(terminate_process_after_error(
                &child.process,
                error,
                "ambient Job membership query",
            ));
        }
    };
    if !inherited_job {
        return Ok(IsolationOutcome::Isolated(child));
    }

    let isolated = match launch_suspended_test_host_with_flags(mode, CREATE_BREAKAWAY_FROM_JOB) {
        Ok(isolated) => isolated,
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            terminate_process_and_wait(&child.process, "ambient-Job test host")?;
            return Ok(IsolationOutcome::AmbientLocked);
        }
        Err(error) => {
            return Err(terminate_process_after_error(
                &child.process,
                error,
                "ambient-Job test host",
            ));
        }
    };
    if let Err(error) = terminate_process_and_wait(&child.process, "ambient-Job test host") {
        return Err(terminate_process_after_error(
            &isolated.process,
            error,
            "isolated breakaway test host",
        ));
    }
    let still_in_job = match process_is_in_any_job(&isolated.process) {
        Ok(still_in_job) => still_in_job,
        Err(error) => {
            return Err(terminate_process_after_error(
                &isolated.process,
                error,
                "breakaway Job membership query",
            ));
        }
    };
    if still_in_job {
        return Err(terminate_process_after_error(
            &isolated.process,
            io::Error::other(
                "CREATE_BREAKAWAY_FROM_JOB left the suspended test host in an ambient Job",
            ),
            "non-isolated breakaway test host",
        ));
    }
    Ok(IsolationOutcome::Isolated(isolated))
}

fn launch_suspended_test_host_with_flags(
    mode: &str,
    extra_flags: u32,
) -> io::Result<SuspendedTestHost> {
    let executable = std::env::current_exe()?;
    let executable_wide: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut command_line: Vec<u16> = format!("se {mode}").encode_utf16().chain(Some(0)).collect();
    let mut startup: STARTUPINFOW = unsafe { zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    if unsafe {
        CreateProcessW(
            executable_wide.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | extra_flags,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut process,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(SuspendedTestHost {
        process: OwnedHandle::new(process.hProcess)?,
        thread: OwnedHandle::new(process.hThread)?,
    })
}

fn process_is_in_any_job(process: &OwnedHandle) -> io::Result<bool> {
    let mut assigned = 0;
    if unsafe { IsProcessInJob(process.raw(), std::ptr::null_mut(), &mut assigned) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(assigned != 0)
    }
}
