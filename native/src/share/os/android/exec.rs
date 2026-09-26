//! Android exec host (`android-subreaper`). The Linux supervisor
//! (`linux_os/exec_supervisor.rs`) runs on a thread of the app process and
//! starts every job below its own subreaper intermediate (`exec_contain.rs`),
//! so cancel, time limit and revocation end the job's whole process tree,
//! `setsid` children and double-forked orphans included, while the app's
//! other children (sync hooks) stay untouched. Shell commands run as
//! `/system/bin/sh -c`; without a working directory a job starts in the
//! host's home directory. Platform limits (`docs/refs/android-child-processes.md`):
//! the commands end with the app process, Android may kill (phantom-process
//! limit) or freeze child processes of a background app, and the host is
//! reachable only while the Share service runs.
#[path = "exec_contain.rs"]
mod exec_contain;
#[path = "../linux_os/exec_supervisor.rs"]
mod exec_supervisor;

use std::ffi::{CStr, OsString};
use std::io;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

use self::exec_contain::{JobRoot, Subreaper};
use self::exec_supervisor::SpawnPolicy;
use crate::share::exec_platform::StopReason;
use crate::share::exec_supervisor_protocol::{
    recv_event, send_command, SupervisorCommand, SupervisorEvent, SupervisorStart,
};
use crate::share::exec_types::{ExecProviderStatus, ExecStart};

const PROVIDER: &str = "android-subreaper";
const SHELL: &CStr = c"/system/bin/sh";
const SHELL_PATH: &str = "/system/bin/sh";
const DETAIL: &str = "Jeder Befehl läuft unter einem eigenen Zwischenprozess (Subreaper) mit \
/system/bin/sh und den Rechten der App; Abbruch, Zeitlimit und Entzug beenden den ganzen \
Prozessbaum. Grenzen von Android: Befehle enden mit der App, im Hintergrund kann Android \
Kindprozesse beenden (ab Android 12 höchstens 32 solcher Prozesse systemweit) oder einfrieren, \
und Befehle kommen nur an, solange der Share-Dienst läuft.";
const DROP_STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// Changes whenever a job of this exec host started running or has ended.
static ACTIVITY: AtomicU64 = AtomicU64::new(0);
static LISTENER: OnceLock<fn()> = OnceLock::new();

pub(crate) struct ContainedExec {
    root: Arc<JobRoot>,
    /// This side of the supervisor socket; shutting it down releases a
    /// supervisor that waits for input or for room to write.
    stream: UnixStream,
    outbound: mpsc::SyncSender<SupervisorCommand>,
    events: mpsc::Receiver<io::Result<SupervisorEvent>>,
    running_seen: bool,
}

impl ContainedExec {
    pub(crate) fn prepare(request: &ExecStart) -> io::Result<Self> {
        exec_contain::recover_once();
        shell_executable()?;
        let root = Arc::new(JobRoot::new(request.exec_id.as_str()));
        let (stream, supervisor_end) = UnixStream::pair()?;
        let supervisor_root = root.clone();
        std::thread::Builder::new()
            .name("exec-supervisor".into())
            .spawn(move || {
                block_sigpipe();
                let _done = SupervisorDone(supervisor_root.clone());
                let subreaper = Subreaper::new(supervisor_root);
                let policy = SpawnPolicy {
                    default_shell: SHELL_PATH,
                    shell_from_environment: false,
                    shell_option: "-c",
                    own_process_group: true,
                    intermediate: Some(&subreaper),
                };
                // A failure without an event reaches the job as a closed
                // supervisor socket.
                let _ = exec_supervisor::run(supervisor_end, &policy);
            })?;
        match Self::connect(root, stream) {
            Ok(process) => Ok(process),
            Err((error, stream)) => {
                let _ = stream.shutdown(Shutdown::Both);
                Err(error)
            }
        }
    }

    /// Reader and writer threads of this side; on failure the socket comes
    /// back so the caller can release the supervisor.
    fn connect(root: Arc<JobRoot>, stream: UnixStream) -> Result<Self, (io::Error, UnixStream)> {
        let (reader, writer) = match (stream.try_clone(), stream.try_clone()) {
            (Ok(reader), Ok(writer)) => (reader, writer),
            (Err(error), _) | (_, Err(error)) => return Err((error, stream)),
        };
        let (event_tx, events) = mpsc::sync_channel(32);
        let (outbound, outbound_rx) = mpsc::sync_channel::<SupervisorCommand>(16);
        let writer_errors = event_tx.clone();
        let spawned = std::thread::Builder::new()
            .name("exec-android-input".into())
            .spawn(move || {
                block_sigpipe();
                let mut writer = writer;
                while let Ok(command) = outbound_rx.recv() {
                    if let Err(error) = send_command(&mut writer, &command) {
                        let _ = writer_errors.send(Err(error));
                        break;
                    }
                }
            })
            .and_then(|_| {
                std::thread::Builder::new()
                    .name("exec-android-events".into())
                    .spawn(move || {
                        let mut reader = reader;
                        loop {
                            let event = recv_event(&mut reader);
                            let terminal = event.is_err();
                            if event_tx.send(event).is_err() || terminal {
                                break;
                            }
                        }
                    })
            });
        if let Err(error) = spawned {
            return Err((error, stream));
        }
        Ok(Self {
            root,
            stream,
            outbound,
            events,
            running_seen: false,
        })
    }

    pub(crate) fn configure(&mut self, _request: &ExecStart) -> io::Result<()> {
        Ok(())
    }

    pub(crate) fn send(&mut self, command: &SupervisorCommand) -> io::Result<()> {
        let command = match command {
            SupervisorCommand::Start(start) => SupervisorCommand::Start(with_host_defaults(start)),
            other => other.clone(),
        };
        self.outbound
            .try_send(command)
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

    pub(crate) fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<SupervisorEvent> {
        let event = match deadline {
            Some(deadline) => self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(recv_timeout_error)?,
            None => self.events.recv().map_err(|_| supervisor_closed())?,
        };
        if matches!(event, Ok(SupervisorEvent::Started { .. })) && !self.running_seen {
            // The registry marks the job running before its first event is read.
            self.running_seen = true;
            note_activity();
        }
        event
    }

    pub(crate) fn terminate_all(&mut self, _reason: StopReason) -> io::Result<()> {
        self.root.terminate();
        // Releases a supervisor that still waits for its start frame, for
        // input or for room to write; its spawn guard then ends the rest.
        let _ = self.stream.shutdown(Shutdown::Both);
        Ok(())
    }

    pub(crate) fn confirm_empty(&mut self, deadline: Instant) -> io::Result<()> {
        self.root.wait_empty(deadline)
    }
}

impl Drop for ContainedExec {
    fn drop(&mut self) {
        let _ = self.terminate_all(StopReason::WorkerStopping);
        let _ = self.confirm_empty(Instant::now() + DROP_STOP_TIMEOUT);
        // The job's terminal state is recorded by now: the change is visible.
        note_activity();
    }
}

/// Marks the job empty once the supervisor thread ends, also on a panic.
struct SupervisorDone(Arc<JobRoot>);

impl Drop for SupervisorDone {
    fn drop(&mut self) {
        self.0.supervisor_done();
    }
}

/// Without a working directory a job starts in the host's home directory
/// (`HOME`, as on the desktop); here-documents of the shell need a writable
/// `TMPDIR`. Values the caller sent explicitly stay untouched.
fn with_host_defaults(start: &SupervisorStart) -> SupervisorStart {
    let mut start = start.clone();
    if let Some(host) = crate::support_dirs::host() {
        if !start.request.env.contains_key("HOME") {
            start
                .environment
                .insert("HOME".into(), host.home_dir.to_string_lossy().into_owned());
        }
        if !start.environment.contains_key("TMPDIR") {
            start.environment.insert(
                "TMPDIR".into(),
                crate::support_dirs::temp_dir()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    start
}

/// Writes to a closed socket or pipe must fail with `EPIPE` on the exec
/// threads: Android does not ignore SIGPIPE, which would end the whole app.
/// Threads started from here inherit the mask; the job's root gets SIGPIPE
/// back in the intermediate hook.
fn block_sigpipe() {
    // SAFETY: changes only this thread's signal mask, with a local set.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGPIPE);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

fn shell_executable() -> io::Result<()> {
    // SAFETY: `SHELL` is a NUL-terminated constant.
    if unsafe { libc::access(SHELL.as_ptr(), libc::X_OK) } == 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{SHELL_PATH} ist nicht ausführbar: {error}"),
        ))
    }
}

fn note_activity() {
    ACTIVITY.fetch_add(1, Ordering::AcqRel);
    if let Some(listener) = LISTENER.get() {
        listener();
    }
}

/// Changes whenever a command of another device started running on this
/// device or has ended (after its terminal state was recorded).
pub(crate) fn exec_host_activity() -> u64 {
    ACTIVITY.load(Ordering::Acquire)
}

/// `listener` runs after every change of [`exec_host_activity`], on the
/// job's thread; it must return quickly. Only the first call takes effect.
pub(crate) fn set_exec_host_listener(listener: fn()) {
    let _ = LISTENER.set(listener);
}

fn recv_timeout_error(error: mpsc::RecvTimeoutError) -> io::Error {
    match error {
        mpsc::RecvTimeoutError::Timeout => {
            io::Error::new(io::ErrorKind::TimedOut, "exec event deadline elapsed")
        }
        mpsc::RecvTimeoutError::Disconnected => supervisor_closed(),
    }
}

fn supervisor_closed() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "exec supervisor closed")
}

pub(crate) fn provider_status() -> ExecProviderStatus {
    exec_contain::recover_once();
    let (available, detail) = match shell_executable() {
        Ok(()) => (true, DETAIL.to_string()),
        Err(error) => (false, error.to_string()),
    };
    ExecProviderStatus {
        available,
        provider: PROVIDER.into(),
        detail,
        elevated: false,
        user_label: "Android-App".into(),
    }
}

/// Android never runs as the hidden supervisor process: the supervisor is a
/// thread of the app.
pub(crate) fn run_supervisor_if_requested(_arguments: &[OsString]) -> Option<io::Result<()>> {
    None
}

/// The platform self-test's generic part covers this provider; containment
/// of detached trees is proven on a device (`ShareExecTaskTest`).
#[cfg(debug_assertions)]
pub(crate) fn run_extended_self_test() -> io::Result<()> {
    Ok(())
}
