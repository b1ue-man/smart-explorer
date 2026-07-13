#[path = "exec_supervisor.rs"]
mod exec_supervisor;

use std::ffi::OsString;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, ResumeThread,
    UpdateProcThreadAttribute, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use crate::share::exec_platform::StopReason;
use crate::share::exec_supervisor_protocol::{
    recv_event, send_command, SupervisorCommand, SupervisorEvent,
};
use crate::share::exec_types::{ExecProviderStatus, ExecStart};

const INTERNAL_MODE: &str = "--share-exec-supervisor";

pub(crate) struct ContainedExec {
    control: Control,
    events: mpsc::Receiver<io::Result<SupervisorEvent>>,
}

struct Control {
    job: OwnedHandle,
    outbound: mpsc::SyncSender<SupervisorCommand>,
    launch: Mutex<LaunchState>,
    stopped: AtomicBool,
}

struct LaunchState {
    process: Option<OwnedHandle>,
    thread: Option<OwnedHandle>,
    resumed: bool,
}

impl ContainedExec {
    pub(crate) fn prepare(_request: &ExecStart) -> io::Result<Self> {
        let job = create_job()?;
        let (child_stdin, writer) = pipe_pair(false)?;
        let (child_stdout, mut reader) = pipe_pair(true)?;
        let launched = launch_supervisor(job.raw(), child_stdin.raw(), child_stdout.raw())?;
        let (event_tx, events) = mpsc::sync_channel(32);
        let (outbound, outbound_rx) = mpsc::sync_channel::<SupervisorCommand>(16);
        let writer_errors = event_tx.clone();
        std::thread::Builder::new()
            .name("exec-job-input".into())
            .spawn(move || {
                let mut writer = writer;
                while let Ok(command) = outbound_rx.recv() {
                    if let Err(error) = send_command(&mut writer, &command) {
                        let _ = writer_errors.send(Err(error));
                        break;
                    }
                }
            })?;
        std::thread::Builder::new()
            .name("exec-job-events".into())
            .spawn(move || loop {
                let event = recv_event(&mut reader);
                let terminal = event.is_err();
                if event_tx.send(event).is_err() || terminal {
                    break;
                }
            })?;
        let control = Control {
            job,
            outbound,
            launch: Mutex::new(LaunchState {
                process: Some(launched.process),
                thread: Some(launched.thread),
                resumed: false,
            }),
            stopped: AtomicBool::new(false),
        };
        Ok(Self { control, events })
    }

    pub(crate) fn configure(&mut self, _request: &ExecStart) -> io::Result<()> {
        Ok(())
    }

    pub(crate) fn send(&mut self, command: &SupervisorCommand) -> io::Result<()> {
        self.control.send(command)
    }

    pub(crate) fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<SupervisorEvent> {
        match deadline {
            Some(deadline) => self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => {
                        io::Error::new(io::ErrorKind::TimedOut, "exec event deadline elapsed")
                    }
                    mpsc::RecvTimeoutError::Disconnected => {
                        io::Error::new(io::ErrorKind::UnexpectedEof, "exec supervisor closed")
                    }
                })?,
            None => self.events.recv().map_err(|_| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "exec supervisor closed")
            })?,
        }
    }

    pub(crate) fn terminate_all(&mut self, _reason: StopReason) -> io::Result<()> {
        self.control.terminate()
    }

    pub(crate) fn confirm_empty(&mut self, deadline: Instant) -> io::Result<()> {
        while Instant::now() < deadline {
            if active_processes(self.control.job.raw())? == 0 {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Windows remote-exec Job still contains processes",
        ))
    }
}

impl Control {
    fn send(&self, command: &SupervisorCommand) -> io::Result<()> {
        if matches!(command, SupervisorCommand::Start(_)) {
            let mut launch = self
                .launch
                .lock()
                .map_err(|_| io::Error::other("exec launch lock poisoned"))?;
            if !launch.resumed {
                let thread = launch
                    .thread
                    .as_ref()
                    .ok_or_else(|| io::Error::other("supervisor thread handle missing"))?;
                if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
                    let error = io::Error::last_os_error();
                    let _ = self.terminate();
                    return Err(error);
                }
                launch.resumed = true;
                launch.thread.take();
                launch.process.take();
            }
        }
        self.outbound
            .try_send(command.clone())
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "exec supervisor input queue is full",
                ),
                mpsc::TrySendError::Disconnected(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "exec supervisor input closed")
                }
            })
    }

    fn terminate(&self) -> io::Result<()> {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if unsafe { TerminateJobObject(self.job.raw(), 1) } == 0 {
            let error = io::Error::last_os_error();
            if active_processes(self.job.raw()).unwrap_or(1) != 0 {
                return Err(error);
            }
        }
        Ok(())
    }
}

impl Drop for ContainedExec {
    fn drop(&mut self) {
        let _ = self.terminate_all(StopReason::WorkerStopping);
        let _ = self.confirm_empty(Instant::now() + Duration::from_secs(5));
    }
}

pub(crate) fn provider_status() -> ExecProviderStatus {
    let available = create_job().map(|_| ()).map_err(|error| error.to_string());
    ExecProviderStatus {
        available: available.is_ok(),
        provider: "Windows Job Object".into(),
        detail: available
            .err()
            .unwrap_or_else(|| "kill-on-close Job Objects are available".into()),
        elevated: process_is_elevated(),
        user_label: std::env::var("USERNAME").unwrap_or_else(|_| "Windows user".into()),
    }
}

pub(crate) fn run_supervisor_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    if arguments.len() == 1 && arguments[0] == INTERNAL_MODE {
        Some(exec_supervisor::run())
    } else {
        None
    }
}

struct LaunchedProcess {
    process: OwnedHandle,
    thread: OwnedHandle,
}

fn create_job() -> io::Result<OwnedHandle> {
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

/// Returns `(child_end, parent_file)`. `parent_reads` chooses pipe direction.
fn pipe_pair(parent_reads: bool) -> io::Result<(OwnedHandle, std::fs::File)> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let read = OwnedHandle::new(read)?;
    let write = OwnedHandle::new(write)?;
    let (child, parent) = if parent_reads {
        (write, read)
    } else {
        (read, write)
    };
    if unsafe { SetHandleInformation(parent.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let raw = parent.into_raw();
    let file = unsafe { std::fs::File::from_raw_handle(raw.cast()) };
    Ok((child, file))
}

fn launch_supervisor(
    job: HANDLE,
    child_stdin: HANDLE,
    child_stdout: HANDLE,
) -> io::Result<LaunchedProcess> {
    let executable = std::env::current_exe()?;
    let executable_wide: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // lpApplicationName selects the exact UTF-16 executable. argv[0] is a
    // fixed placeholder, so no lossy path conversion or arbitrary quoting is
    // involved in this pre-authorization launch.
    let mut command_line_wide: Vec<u16> = format!("se {INTERNAL_MODE}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut attributes = AttributeList::new(2)?;
    let inherited = [child_stdin, child_stdout];
    attributes.set(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        inherited.as_ptr().cast(),
        size_of_val(&inherited),
    )?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
        (&job as *const HANDLE).cast(),
        size_of::<HANDLE>(),
    )?;
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = child_stdin;
    startup.StartupInfo.hStdOutput = child_stdout;
    startup.StartupInfo.hStdError = child_stdout;
    startup.lpAttributeList = attributes.ptr();
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    let created = unsafe {
        CreateProcessW(
            executable_wide.as_ptr(),
            command_line_wide.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT
                | CREATE_UNICODE_ENVIRONMENT
                | CREATE_SUSPENDED
                | CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &startup.StartupInfo,
            &mut process,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(LaunchedProcess {
        process: OwnedHandle::new(process.hProcess)?,
        thread: OwnedHandle::new(process.hThread)?,
    })
}

fn active_processes(job: HANDLE) -> io::Result<u32> {
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

fn process_is_elevated() -> bool {
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let token = match OwnedHandle::new(token) {
        Ok(token) => token,
        Err(_) => return false,
    };
    let mut elevation: TOKEN_ELEVATION = unsafe { zeroed() };
    let mut returned = 0;
    let ok = unsafe {
        GetTokenInformation(
            token.raw(),
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    ok != 0 && elevation.TokenIsElevated != 0
}

struct AttributeList {
    bytes: Vec<usize>,
    initialized: bool,
}

impl AttributeList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0usize;
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let words = size.div_ceil(size_of::<usize>());
        let mut value = Self {
            bytes: vec![0; words],
            initialized: false,
        };
        if unsafe { InitializeProcThreadAttributeList(value.ptr(), count, 0, &mut size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        value.initialized = true;
        Ok(value)
    }

    fn ptr(&mut self) -> windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST {
        self.bytes.as_mut_ptr().cast()
    }

    fn set(
        &mut self,
        attribute: usize,
        value: *const core::ffi::c_void,
        size: usize,
    ) -> io::Result<()> {
        if unsafe {
            UpdateProcThreadAttribute(
                self.ptr(),
                0,
                attribute,
                value,
                size,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { DeleteProcThreadAttributeList(self.ptr()) };
        }
    }
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE) -> io::Result<Self> {
        if handle.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }

    fn into_raw(mut self) -> HANDLE {
        let raw = self.0;
        self.0 = std::ptr::null_mut();
        raw
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_creates_an_empty_kill_on_close_job() {
        let job = create_job().unwrap();
        assert_eq!(active_processes(job.raw()).unwrap(), 0);
    }
}
