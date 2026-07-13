use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_JOB_LIST,
    STARTUPINFOEXW, STARTUPINFOW,
};

use super::super::{AttributeList, OwnedHandle};
use super::terminate_process_after_error;

pub(super) struct SuspendedTestHost {
    pub(super) process: OwnedHandle,
    pub(super) thread: OwnedHandle,
}

pub(super) fn launch_suspended_test_host(mode: &str) -> io::Result<SuspendedTestHost> {
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
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
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

pub(super) fn launch_suspended_test_host_in_job(
    mode: &str,
    job: HANDLE,
) -> io::Result<SuspendedTestHost> {
    let executable = std::env::current_exe()?;
    let executable_wide: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut command_line: Vec<u16> = format!("se {mode}").encode_utf16().chain(Some(0)).collect();
    let mut attributes = AttributeList::new(1)?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
        (&job as *const HANDLE).cast(),
        size_of::<HANDLE>(),
    )?;
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.lpAttributeList = attributes.ptr();
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    if unsafe {
        CreateProcessW(
            executable_wide.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT
                | CREATE_SUSPENDED
                | CREATE_NO_WINDOW
                | CREATE_UNICODE_ENVIRONMENT,
            std::ptr::null(),
            std::ptr::null(),
            &startup.StartupInfo,
            &mut process,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let child = SuspendedTestHost {
        process: OwnedHandle::new(process.hProcess)?,
        thread: OwnedHandle::new(process.hThread)?,
    };
    let mut assigned = 0;
    if unsafe { IsProcessInJob(child.process.raw(), job, &mut assigned) } == 0 {
        let error = io::Error::last_os_error();
        return Err(terminate_process_after_error(
            &child.process,
            error,
            "restricted outer Job membership query",
        ));
    }
    if assigned == 0 {
        return Err(terminate_process_after_error(
            &child.process,
            io::Error::other("suspended test host is not in its restricted outer Job"),
            "unassigned restricted outer Job test host",
        ));
    }
    Ok(child)
}
