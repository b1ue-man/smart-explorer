//! The process tree of a started command: a job object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The command starts suspended
//! (`CREATE_SUSPENDED`), joins the job and only then runs, so no process it
//! starts escapes the job; its threads are resumed through a thread
//! snapshot, because std does not hand out the main thread's handle.
use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::ptr::null;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

/// Exit status of the processes a cancel ends (`STATUS_CONTROL_C_EXIT`).
const CANCEL_EXIT: u32 = 0xC000_013A;

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: an owned handle, closed once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub struct Tree {
    job: Handle,
}

// SAFETY: a job handle may be used from any thread.
unsafe impl Send for Tree {}
// SAFETY: TerminateJobObject is safe to call concurrently.
unsafe impl Sync for Tree {}

impl Tree {
    /// Puts the suspended `child` into a new job and resumes it.
    pub fn adopt(child: &Child) -> io::Result<Self> {
        // SAFETY: an unnamed job with default security.
        let job = unsafe { CreateJobObjectW(null(), null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Handle(job);
        // SAFETY: a zeroed limit structure with one flag set.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the structure and its size.
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the child's process handle, valid while `child` lives.
        if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        resume_threads(child.id())?;
        Ok(Self { job })
    }

    /// Ends every process of the tree (Windows has no gentle tree signal).
    pub fn terminate(&self) {
        self.kill();
    }

    pub fn kill(&self) {
        // SAFETY: the job handle this tree owns.
        unsafe {
            TerminateJobObject(self.job.0, CANCEL_EXIT);
        }
    }
}

fn resume_threads(pid: u32) -> io::Result<()> {
    // SAFETY: a snapshot of every thread of the system.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = Handle(snapshot);
    // SAFETY: dwSize set as the API requires.
    let mut entry: THREADENTRY32 = unsafe { zeroed() };
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    let mut resumed = 0;
    // SAFETY: the snapshot and an entry of the declared size.
    let mut more = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: opening a thread by id for resuming it.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                let thread = Handle(thread);
                // SAFETY: a thread handle with THREAD_SUSPEND_RESUME.
                if unsafe { ResumeThread(thread.0) } != u32::MAX {
                    resumed += 1;
                }
            }
        }
        // SAFETY: as for Thread32First.
        more = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
    }
    if resumed == 0 {
        return Err(io::Error::other(
            "the started command's thread could not be resumed",
        ));
    }
    Ok(())
}
