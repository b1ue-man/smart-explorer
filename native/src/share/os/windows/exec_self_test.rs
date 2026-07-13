use std::ffi::OsString;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::time::{Duration, Instant};

#[path = "exec_self_test_process.rs"]
mod process;

use self::process::{
    launch_isolated_suspended_test_host, launch_suspended_test_host, IsolationOutcome,
    SuspendedTestHost,
};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, IsProcessInJob, JobObjectBasicUIRestrictions,
    SetInformationJobObject, JOBOBJECT_BASIC_UI_RESTRICTIONS, JOB_OBJECT_UILIMIT_HANDLES,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateProcessW, GetExitCodeProcess, ResumeThread, SetEvent, TerminateProcess,
    WaitForSingleObject, CREATE_BREAKAWAY_FROM_JOB, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
    PROCESS_INFORMATION, STARTUPINFOW,
};

use crate::share::exec_supervisor_protocol::{
    environment_for, SupervisorCommand, SupervisorEvent, SupervisorStart,
};
use crate::share::exec_types::{ExecCommand, ExecId, ExecStart};

use super::{ContainedExec, OwnedHandle, StopReason};

const NESTED_HELPER_MODE: &str = "--share-exec-windows-nested-job-probe";
const INCOMPATIBLE_HELPER_MODE: &str = "--share-exec-windows-incompatible-job-probe";
const BREAKAWAY_HELPER_MODE: &str = "--share-exec-windows-breakaway-probe";
const BREAKAWAY_TARGET_MODE: &str = "--share-exec-windows-breakaway-target";
const HANDLE_HELPER_MODE: &str = "--share-exec-windows-handle-probe";

pub(super) fn run_helper_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    if arguments.len() == 1 {
        let result = if arguments[0] == NESTED_HELPER_MODE {
            run_nested_helper()
        } else if arguments[0] == INCOMPATIBLE_HELPER_MODE {
            run_incompatible_helper()
        } else if arguments[0] == BREAKAWAY_HELPER_MODE {
            run_breakaway_helper()
        } else if arguments[0] == BREAKAWAY_TARGET_MODE {
            Ok(())
        } else {
            return None;
        };
        return Some(result);
    }
    if arguments.len() == 2 && arguments[0] == HANDLE_HELPER_MODE {
        return Some(run_handle_helper(&arguments[1]));
    }
    None
}

pub(super) fn run() -> io::Result<()> {
    run_compatible_outer_job_test()?;
    run_incompatible_outer_job_test()?;
    run_breakaway_test()?;
    run_handle_allowlist_test()
}

fn run_compatible_outer_job_test() -> io::Result<()> {
    let outer = super::create_job()?;
    // Keep an ambient runner Job when one exists. Adding our unrestricted
    // outer Job then exercises the same supported nesting used by production.
    let child = launch_suspended_test_host(NESTED_HELPER_MODE)?;
    assign_to_outer_job(&outer, &child)?;
    resume_and_expect_success(child)?;
    require_empty_job(&outer, "compatible outer Job")
}

fn run_incompatible_outer_job_test() -> io::Result<()> {
    let outer = create_ui_restricted_job()?;
    match launch_isolated_suspended_test_host(INCOMPATIBLE_HELPER_MODE)? {
        IsolationOutcome::Isolated(child) => run_isolated_incompatible_test(&outer, child),
        IsolationOutcome::AmbientLocked => require_ambient_incompatible_rejection(&outer),
    }
}

fn run_isolated_incompatible_test(outer: &OwnedHandle, child: SuspendedTestHost) -> io::Result<()> {
    if unsafe { AssignProcessToJobObject(outer.raw(), child.process.raw()) } == 0 {
        let error = io::Error::last_os_error();
        return Err(terminate_process_after_error(
            &child.process,
            error,
            "incompatible outer Job setup",
        ));
    }
    resume_and_expect_success(child)?;
    require_empty_job(outer, "UI-restricted outer Job")
}

fn require_ambient_incompatible_rejection(outer: &OwnedHandle) -> io::Result<()> {
    let (child_stdin, _writer) = super::pipe_pair(false)?;
    let (child_stdout, _reader) = super::pipe_pair(true)?;
    match super::launch_supervisor(outer.raw(), child_stdin.raw(), child_stdout.raw()) {
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => Ok(()),
        Err(error) => Err(io::Error::other(format!(
            "UI-restricted production launch failed with an unexpected error: {error}"
        ))),
        Ok(launched) => Err(terminate_process_after_error(
            &launched.process,
            io::Error::other(
                "production launcher unexpectedly accepted a UI-restricted nested Job",
            ),
            "unexpected incompatible production launch",
        )),
    }
}

fn run_breakaway_test() -> io::Result<()> {
    run_contained_helper(vec![BREAKAWAY_HELPER_MODE.into()])
}

fn run_handle_allowlist_test() -> io::Result<()> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let event = OwnedHandle::new(unsafe { CreateEventW(&attributes, 1, 0, std::ptr::null()) })?;
    run_contained_helper(vec![
        HANDLE_HELPER_MODE.into(),
        (event.raw() as usize).to_string(),
    ])?;
    match unsafe { WaitForSingleObject(event.raw(), 0) } {
        WAIT_TIMEOUT => Ok(()),
        WAIT_OBJECT_0 => Err(io::Error::other(
            "unexpected inheritable event escaped the supervisor handle allowlist",
        )),
        _ => Err(io::Error::last_os_error()),
    }
}

fn run_nested_helper() -> io::Result<()> {
    run_contained_helper(vec![BREAKAWAY_TARGET_MODE.into()])
}

fn run_incompatible_helper() -> io::Result<()> {
    let request = helper_request(vec![BREAKAWAY_TARGET_MODE.into()])?;
    match ContainedExec::prepare(&request) {
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => Ok(()),
        Err(error) => Err(io::Error::other(format!(
            "UI-restricted nested Job failed with an unexpected error: {error}"
        ))),
        Ok(mut process) => {
            process.terminate_all(StopReason::ProtocolError)?;
            process.confirm_empty(Instant::now() + Duration::from_secs(5))?;
            Err(io::Error::other(
                "UI-restricted nested Job unexpectedly accepted the supervisor",
            ))
        }
    }
}

fn run_breakaway_helper() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let executable_wide: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut command_line: Vec<u16> = format!("se {BREAKAWAY_TARGET_MODE}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut startup: STARTUPINFOW = unsafe { zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    let created = unsafe {
        CreateProcessW(
            executable_wide.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut process,
        )
    };
    if created != 0 {
        let process_handle = OwnedHandle::new(process.hProcess)?;
        let _thread_handle = OwnedHandle::new(process.hThread)?;
        terminate_process_and_wait(&process_handle, "escaped breakaway target")?;
        return Err(io::Error::other(
            "CREATE_BREAKAWAY_FROM_JOB escaped the remote-exec Job",
        ));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
        Ok(())
    } else {
        Err(error)
    }
}

fn run_handle_helper(argument: &OsString) -> io::Result<()> {
    let value = argument
        .to_string_lossy()
        .parse::<usize>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid test handle"))?;
    if unsafe { SetEvent(value as HANDLE) } != 0 {
        return Err(io::Error::other(
            "unexpected inheritable handle was available in the payload",
        ));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_INVALID_HANDLE as i32) {
        Ok(())
    } else {
        Err(error)
    }
}

fn run_contained_helper(arguments: Vec<String>) -> io::Result<()> {
    let request = helper_request(arguments)?;
    request.validate()?;
    let environment = environment_for(&request);
    let mut process = ContainedExec::prepare(&request)?;
    require_owned_supervisor_job(&process)?;
    process.configure(&request)?;
    process.send(&SupervisorCommand::Start(SupervisorStart {
        request,
        environment,
    }))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut root_exited = false;
    loop {
        match process.next_event(Some(deadline))? {
            SupervisorEvent::RootExited(exit) => {
                if exit.code != Some(0) {
                    return Err(io::Error::other(format!(
                        "Windows containment helper failed: {exit:?}"
                    )));
                }
                root_exited = true;
            }
            SupervisorEvent::Exited(exit) => {
                if !root_exited || exit.code != Some(0) {
                    return Err(io::Error::other(format!(
                        "Windows containment helper exit was incomplete: {exit:?}"
                    )));
                }
                break;
            }
            SupervisorEvent::Error(message) => return Err(io::Error::other(message)),
            _ => {}
        }
    }
    process.terminate_all(StopReason::RootExited)?;
    process.confirm_empty(Instant::now() + Duration::from_secs(10))
}

fn helper_request(arguments: Vec<String>) -> io::Result<ExecStart> {
    let executable = std::env::current_exe()?
        .into_os_string()
        .into_string()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-Unicode test executable"))?;
    Ok(ExecStart {
        exec_id: ExecId::generate()?,
        command: ExecCommand::Argv {
            program: executable,
            args: arguments,
        },
        cwd: None,
        env: Default::default(),
        timeout_ms: Some(10_000),
        max_output_bytes: None,
    })
}

fn create_ui_restricted_job() -> io::Result<OwnedHandle> {
    let job = super::create_job()?;
    let restrictions = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_HANDLES,
    };
    if unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectBasicUIRestrictions,
            (&restrictions as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
            size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

fn assign_to_outer_job(job: &OwnedHandle, child: &SuspendedTestHost) -> io::Result<()> {
    if unsafe { AssignProcessToJobObject(job.raw(), child.process.raw()) } == 0 {
        let error = io::Error::last_os_error();
        return Err(terminate_process_after_error(
            &child.process,
            error,
            "outer Job assignment",
        ));
    }
    let mut assigned = 0;
    if unsafe { IsProcessInJob(child.process.raw(), job.raw(), &mut assigned) } == 0 {
        let error = io::Error::last_os_error();
        return Err(terminate_process_after_error(
            &child.process,
            error,
            "outer Job membership query",
        ));
    }
    if assigned == 0 {
        return Err(terminate_process_after_error(
            &child.process,
            io::Error::other("test host was not assigned to its outer Job"),
            "unassigned outer Job test host",
        ));
    }
    Ok(())
}

fn resume_and_expect_success(child: SuspendedTestHost) -> io::Result<()> {
    if unsafe { ResumeThread(child.thread.raw()) } == u32::MAX {
        let error = io::Error::last_os_error();
        return Err(terminate_process_after_error(
            &child.process,
            error,
            "resume nested Job test host",
        ));
    }
    match unsafe { WaitForSingleObject(child.process.raw(), 30_000) } {
        WAIT_OBJECT_0 => {}
        WAIT_TIMEOUT => {
            return Err(terminate_process_after_error(
                &child.process,
                io::Error::new(io::ErrorKind::TimedOut, "nested Job test host did not exit"),
                "timed-out nested Job test host",
            ));
        }
        _ => {
            let error = io::Error::last_os_error();
            return Err(terminate_process_after_error(
                &child.process,
                error,
                "failed nested Job test wait",
            ));
        }
    }
    let mut exit_code = 0;
    if unsafe { GetExitCodeProcess(child.process.raw(), &mut exit_code) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if exit_code == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "nested Job test host exited with {exit_code}"
        )))
    }
}

fn require_empty_job(job: &OwnedHandle, label: &str) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if super::active_processes(job.raw())? == 0 {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("{label} did not drain"),
    ))
}

fn require_owned_supervisor_job(process: &ContainedExec) -> io::Result<()> {
    let launch = process
        .control
        .launch
        .lock()
        .map_err(|_| io::Error::other("exec launch lock poisoned during containment probe"))?;
    let supervisor = launch
        .process
        .as_ref()
        .ok_or_else(|| io::Error::other("suspended exec supervisor handle is missing"))?;
    let mut assigned = 0;
    if unsafe { IsProcessInJob(supervisor.raw(), process.control.job.raw(), &mut assigned) } == 0 {
        let error = io::Error::last_os_error();
        return Err(terminate_process_after_error(
            supervisor,
            error,
            "Smart Explorer Job membership query",
        ));
    }
    if assigned == 0 {
        return Err(terminate_process_after_error(
            supervisor,
            io::Error::other("exec supervisor is not in the Smart Explorer Job"),
            "uncontained exec supervisor",
        ));
    }
    Ok(())
}

fn terminate_process_after_error(
    process: &OwnedHandle,
    primary: io::Error,
    label: &str,
) -> io::Error {
    match terminate_process_and_wait(process, label) {
        Ok(()) => primary,
        Err(cleanup) => {
            io::Error::other(format!("{primary}; {label} cleanup also failed: {cleanup}"))
        }
    }
}

fn terminate_process_and_wait(process: &OwnedHandle, label: &str) -> io::Result<()> {
    match unsafe { WaitForSingleObject(process.raw(), 0) } {
        WAIT_OBJECT_0 => return Ok(()),
        WAIT_TIMEOUT => {}
        _ => return Err(io::Error::last_os_error()),
    }
    if unsafe { TerminateProcess(process.raw(), 1) } == 0 {
        let terminate_error = io::Error::last_os_error();
        return match unsafe { WaitForSingleObject(process.raw(), 0) } {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(terminate_error),
            _ => Err(io::Error::other(format!(
                "{terminate_error}; {label} state check failed: {}",
                io::Error::last_os_error()
            ))),
        };
    }
    match unsafe { WaitForSingleObject(process.raw(), 5_000) } {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{label} did not terminate"),
        )),
        _ => Err(io::Error::last_os_error()),
    }
}
