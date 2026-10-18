// The account service of PRM-M7: worker side, service, setup.
#[path = "exec_account_choices.rs"]
mod exec_account_choices;
#[path = "exec_account_client.rs"]
mod exec_account_client;
#[path = "exec_account_config.rs"]
mod exec_account_config;
#[path = "exec_account_connection.rs"]
mod exec_account_connection;
#[path = "exec_account_files.rs"]
mod exec_account_files;
#[path = "exec_account_install.rs"]
mod exec_account_install;
#[cfg(test)]
#[path = "exec_account_job_tests.rs"]
mod exec_account_job_tests;
#[path = "exec_account_pipes.rs"]
mod exec_account_pipes;
#[path = "exec_account_s4u.rs"]
mod exec_account_s4u;
#[path = "exec_account_scm.rs"]
mod exec_account_scm;
#[path = "exec_account_security.rs"]
mod exec_account_security;
#[path = "exec_account_service.rs"]
mod exec_account_service;
#[path = "exec_account_setup.rs"]
mod exec_account_setup;
#[path = "exec_account_spawn.rs"]
mod exec_account_spawn;
#[cfg(test)]
#[path = "exec_account_tests.rs"]
mod exec_account_tests;
#[cfg(test)]
#[path = "exec_account_unit_tests.rs"]
mod exec_account_unit_tests;
#[path = "exec_conpty.rs"]
mod exec_conpty;
#[path = "exec_handles.rs"]
mod exec_handles;
#[path = "exec_job_object.rs"]
mod exec_job_object;
#[cfg(debug_assertions)]
#[path = "exec_self_test.rs"]
mod exec_self_test;
#[path = "exec_supervisor.rs"]
mod exec_supervisor;

pub(crate) use self::exec_account_choices::account_choices;
pub(crate) use self::exec_account_client::{resolve_account, run_as_supported};
pub(crate) use self::exec_account_setup::{
    exec_accounts_apply, Elevation, ExecAccountsAction, ExecAccountsOutcome,
};

use std::ffi::OsString;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use self::exec_account_client::AccountSession;
use self::exec_account_security::process_is_elevated;
use self::exec_handles::{AttributeList, OwnedHandle};
use self::exec_job_object::{active_processes, create_job, terminate_job};
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

/// Where a job's supervisor runs, and how it is ended and checked.
enum Containment {
    /// The worker's own account: the worker's kill-on-close Job, exactly as
    /// before PRM-M7.
    Own(OwnedHandle),
    /// Another account (PRM-M7): a session of the account service, whose
    /// Job in session 0 holds the relay and its children.
    Account(Mutex<AccountSession>),
}

struct Control {
    containment: Containment,
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
    /// Prepares the supervisor of `request`: as the worker's own account,
    /// or as the account `exec_platform` prepares the start for on this
    /// thread (`ContainedExec::prepare_as`) through the account service.
    pub(crate) fn prepare(request: &ExecStart) -> io::Result<Self> {
        if let Some(account) = crate::share::exec_platform::preparing_account() {
            let (session, writer, reader) = AccountSession::start(request, &account)?;
            // The service starts the relay running; it waits for its start.
            let launch = LaunchState {
                process: None,
                thread: None,
                resumed: true,
            };
            let containment = Containment::Account(Mutex::new(session));
            return Self::connected(containment, writer, reader, launch);
        }
        let job = create_job()?;
        let (child_stdin, writer) = pipe_pair(false)?;
        let (child_stdout, reader) = pipe_pair(true)?;
        let launched = launch_supervisor(job.raw(), child_stdin.raw(), child_stdout.raw())?;
        let launch = LaunchState {
            process: Some(launched.process),
            thread: Some(launched.thread),
            resumed: false,
        };
        Self::connected(Containment::Own(job), writer, reader, launch)
    }

    /// Starts the two threads between the worker and the supervisor: one
    /// writes the commands, one reads the events.
    fn connected(
        containment: Containment,
        writer: std::fs::File,
        mut reader: std::fs::File,
        launch: LaunchState,
    ) -> io::Result<Self> {
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
            containment,
            outbound,
            launch: Mutex::new(launch),
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
        let poll = match self.control.containment {
            Containment::Own(_) => Duration::from_millis(10),
            Containment::Account(_) => Duration::from_millis(50),
        };
        while Instant::now() < deadline {
            if self.control.empty()? {
                return Ok(());
            }
            std::thread::sleep(poll);
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
        match &self.containment {
            Containment::Own(job) => terminate_job(job.raw()),
            Containment::Account(session) => lock_session(session)?.stop(),
        }
    }

    /// No process of the job is left.
    fn empty(&self) -> io::Result<bool> {
        match &self.containment {
            Containment::Own(job) => Ok(active_processes(job.raw())? == 0),
            Containment::Account(session) => lock_session(session)?.finished(),
        }
    }
}

fn lock_session(session: &Mutex<AccountSession>) -> io::Result<MutexGuard<'_, AccountSession>> {
    session
        .lock()
        .map_err(|_| io::Error::other("the account service session lock is poisoned"))
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

/// True when this Windows has the pseudoconsole (ConPTY, Windows 10 1809 and
/// newer), so an Exec start may bring a terminal. Resolved once; cheap.
pub(crate) fn terminal_supported() -> bool {
    exec_conpty::terminal_supported()
}

/// Lookup path of a grant with a program list: the Windows system
/// directories below the worker's own `SystemRoot`, fixed, never the
/// caller's `PATH`. Only `.exe` and `.com` complete a bare name; command
/// scripts would start a shell.
pub(crate) fn program_search() -> crate::share::exec_policy::program_path::ProgramSearch {
    let root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    crate::share::exec_policy::program_path::ProgramSearch {
        directories: vec![
            root.join("System32"),
            root.clone(),
            root.join(r"System32\Wbem"),
        ],
        suffixes: &[".exe", ".com"],
    }
}

/// The hidden modes of this program on Windows: the supervisor of a job,
/// and the account service and its elevated setup (PRM-M7).
pub(crate) fn run_supervisor_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    #[cfg(debug_assertions)]
    if let Some(result) = exec_self_test::run_helper_if_requested(arguments) {
        return Some(result);
    }
    let hidden = exec_account_service::run_if_requested(arguments)
        .or_else(|| exec_account_setup::run_if_requested(arguments));
    if hidden.is_some() {
        return hidden;
    }
    if arguments.len() == 1 && arguments[0] == INTERNAL_MODE {
        Some(exec_supervisor::run())
    } else {
        None
    }
}

#[cfg(debug_assertions)]
pub(crate) fn run_extended_self_test() -> io::Result<()> {
    exec_self_test::run()
}

struct LaunchedProcess {
    process: OwnedHandle,
    thread: OwnedHandle,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_creates_an_empty_kill_on_close_job() {
        let job = create_job().unwrap();
        assert_eq!(active_processes(job.raw()).unwrap(), 0);
    }
}
