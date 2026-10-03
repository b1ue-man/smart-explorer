//! cmd quoting and a kill-on-close job containing the shell and descendants.
use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, THREADENTRY32, TH32CS_SNAPTHREAD};

pub(crate) fn shell_command(script: &str) -> Command {
    let mut command = Command::new("cmd");
    // Rust's ordinary Windows argv escaping changes embedded quotes for
    // cmd. Pass the command string using cmd's /s outer-quote convention.
    command.args(["/d", "/s", "/c"])
        .raw_arg(format!("\"{script}\""))
        .creation_flags(CREATE_NO_WINDOW);
    command
}

pub(crate) struct ShellChild { child: Child, job: HANDLE }

pub(crate) fn spawn_shell(mut command: Command) -> io::Result<ShellChild> {
    // SAFETY: creates an unnamed job with no inherited handles.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() { return Err(io::Error::last_os_error()); }
    // SAFETY: zeroed extended limit structure with only the valid flag set.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let set = unsafe { SetInformationJobObject(job, JobObjectExtendedLimitInformation,
        (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
        std::mem::size_of_val(&limits) as u32) };
    if set == 0 {
        let error = io::Error::last_os_error();
        unsafe { CloseHandle(job); }
        return Err(error);
    }
    command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => { unsafe { CloseHandle(job); } return Err(error); }
    };
    if unsafe { AssignProcessToJobObject(job, child.as_raw_handle()) } == 0 {
        let error = io::Error::last_os_error();
        let _ = child.kill();
        let _ = child.wait();
        unsafe { CloseHandle(job); }
        return Err(error);
    }
    // Resume only after containment: no hook can create a descendant in
    // the window between spawn and AssignProcessToJobObject.
    if let Err(error) = resume(child.id()) {
        unsafe { CloseHandle(job); }
        let _ = child.kill(); let _ = child.wait();
        return Err(error);
    }
    Ok(ShellChild { child, job })
}

impl ShellChild {
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> { self.child.try_wait() }
    pub(crate) fn stop(&mut self) {
        if !self.job.is_null() {
            unsafe { CloseHandle(self.job); }
            self.job = std::ptr::null_mut();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for ShellChild { fn drop(&mut self) { self.stop(); } }

fn resume(pid: u32) -> io::Result<()> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE { return Err(io::Error::last_os_error()); }
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut found = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    let mut result = Err(io::Error::new(io::ErrorKind::NotFound, "suspended shell thread not found"));
    while found {
        if entry.th32OwnerProcessID == pid {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() { result = Err(io::Error::last_os_error()); break; }
            let resumed = unsafe { ResumeThread(thread) };
            result = if resumed == u32::MAX { Err(io::Error::last_os_error()) } else { Ok(()) };
            unsafe { CloseHandle(thread); }
            break;
        }
        found = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot); }
    result
}
