//! The daemon side of the screen host process (SCR-M3): every session opens
//! its source in a process of its own that captures and encodes there
//! (`--screen-host-worker`), so a crash in capture, Media Foundation or
//! OpenH264 ends only that session. The process is the desktop program next
//! to this one (KWin matches its window rights by that path, A6); without
//! it only the test picture runs in this program itself. One long-lived
//! thread starts every host process (on Linux the parent-death signal
//! belongs to the starting thread). Each request waits a bounded time for
//! its answer; a process that died, ended for an update (exit 75, V8) or
//! broke the protocol ends the session with `host_shutdown` or
//! `capture_failed`. Dropping the source sends `CLOSE`, waits briefly and
//! kills what is left.
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::{ScreenSource, SourceFrame, SourceRefusal};
use crate::screen::core::host_stage::{HostStage, StageError, StageFrame, StageRequest};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_hello::ScreenErrorCode;
use crate::screen::core::worker_wire::{send_request, OpenRequest, WorkerReply, WorkerRequest};

#[path = "host_worker_reader.rs"]
mod reader;
use reader::WorkerReader;
#[path = "host_worker_backend.rs"]
mod backend;
#[path = "capture_broker.rs"]
mod capture_broker;
#[path = "host_worker_lifetime.rs"]
mod capture_lifetime;
#[path = "family_backend.rs"]
pub(crate) mod family_backend;
pub(crate) use backend::{watch_registry, WorkerBackend, WorkerProgram};

/// The V9 start switch of the host process (`StartRequest::ScreenHostWorker`).
pub(crate) const SCREEN_HOST_WORKER_SWITCH: &str = "--screen-host-worker";
/// Process id of the daemon, for the parent check of the host process.
pub(crate) const PARENT_ENV: &str = "SE_SCREEN_WORKER_PARENT";
/// Time to open a source; a portal dialog on the host may take a while.
const OPEN_TIMEOUT: Duration = Duration::from_secs(120);
/// Time for one picture (the first one starts the encoder).
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// Time for one input batch (SCR-M6), the wait for a running picture
/// request included.
pub(super) const INPUT_TIMEOUT: Duration = Duration::from_secs(2);
/// Time a closed host process gets to exit before it is killed.
const EXIT_WAIT: Duration = Duration::from_secs(2);

struct SpawnJob {
    command: Command,
    started: mpsc::Sender<io::Result<Child>>,
}

static SPAWNER: Mutex<Option<mpsc::Sender<SpawnJob>>> = Mutex::new(None);

fn start_spawner() -> io::Result<mpsc::Sender<SpawnJob>> {
    let (jobs, queue) = mpsc::channel::<SpawnJob>();
    std::thread::Builder::new()
        .name("screen-host-spawner".into())
        .spawn(move || {
            for mut job in queue {
                let _ = job.started.send(job.command.spawn());
            }
        })?;
    Ok(jobs)
}

/// Starts `command` from the long-lived spawner thread.
fn spawn(command: Command) -> io::Result<Child> {
    let (started, answer) = mpsc::channel();
    let mut job = SpawnJob { command, started };
    let mut slot = SPAWNER.lock().unwrap_or_else(PoisonError::into_inner);
    for _ in 0..2 {
        let sender = match slot.as_ref() {
            Some(sender) => sender.clone(),
            None => {
                let sender = start_spawner()?;
                *slot = Some(sender.clone());
                sender
            }
        };
        match sender.send(job) {
            Ok(()) => {
                drop(slot);
                return answer
                    .recv()
                    .map_err(|_| io::Error::other("the screen host spawner ended"))?;
            }
            Err(mpsc::SendError(returned)) => {
                job = returned;
                *slot = None;
            }
        }
    }
    Err(io::Error::other("the screen host spawner does not start"))
}

/// Waits up to `limit` for the exit of `child`.
fn wait_for(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => return None,
        }
    }
}

struct SourceFacts {
    generation: AtomicU64,
    regions: AtomicBool,
    limits: Mutex<FrameLimits>,
    family_layout_revision: AtomicU64,
    legacy_reconnect: AtomicBool,
}
impl Default for SourceFacts {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(0),
            regions: AtomicBool::new(true),
            limits: Mutex::new(FrameLimits::LEGACY),
            family_layout_revision: AtomicU64::new(0),
            legacy_reconnect: AtomicBool::new(false),
        }
    }
}

/// One running host process.
pub(super) struct WorkerLink {
    facts: Arc<SourceFacts>,
    child: Option<Child>,
    input: Arc<Mutex<ChildStdin>>,
    broker: Arc<capture_broker::CaptureBroker>,
    reader: WorkerReader,
}

impl WorkerLink {
    fn start(_program: &Path, _env: &[(String, String)]) -> io::Result<Self> {
        Err(io::Error::other(
            "worker opening requires a prepared session admission",
        ))
    }
    fn start_prepared(
        program: &Path,
        env: &[(String, String)],
        broker: Arc<capture_broker::CaptureBroker>,
    ) -> io::Result<Self> {
        let mut command = Command::new(program);
        command.envs(env.iter().map(|(key, value)| (key, value)));
        command
            .arg(SCREEN_HOST_WORKER_SWITCH)
            .env(PARENT_ENV, std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        super::worker_lifetime::prepare(&mut command);
        Self::with_command_admitted(command, broker)
    }

    fn with_command(_command: Command) -> io::Result<Self> {
        Err(io::Error::other(
            "worker command requires a prepared session admission",
        ))
    }
    fn with_command_admitted(
        command: Command,
        broker: Arc<capture_broker::CaptureBroker>,
    ) -> io::Result<Self> {
        let mut child = spawn(command)?;
        if let Err(error) = super::worker_lifetime::bind(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("screen host process has no pipes"));
        };
        let input = Arc::new(Mutex::new(input));
        let reader = match WorkerReader::start(output, input.clone(), broker.clone()) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            facts: Arc::new(SourceFacts::default()),
            child: Some(child),
            input,
            broker,
            reader,
        })
    }

    /// Sends `request` and waits up to `limit` for the answer.
    pub(super) fn ask(
        &mut self,
        request: &WorkerRequest,
        limit: Duration,
    ) -> Result<WorkerReply, StageError> {
        let sent = self
            .input
            .lock()
            .map_err(|_| io::Error::other("worker input poisoned"))
            .and_then(|mut input| send_request(&mut *input, request));
        if let Err(error) = sent {
            return Err(self.lost(error));
        }
        match self.reader.receive(limit) {
            Ok(Ok(Some(reply))) => {
                let reply = if let WorkerReply::Reference {
                    generation,
                    regions,
                    limits,
                    family_layout_revision,
                    reply,
                } = reply
                {
                    if generation < self.facts.generation.load(Ordering::Acquire) {
                        self.kill();
                        return Err(StageError::new(
                            ScreenErrorCode::CaptureFailed,
                            io::Error::other("worker source generation regressed"),
                        ));
                    }
                    *self
                        .facts
                        .limits
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = limits;
                    self.facts
                        .family_layout_revision
                        .store(family_layout_revision.unwrap_or(0), Ordering::Release);
                    self.facts.regions.store(regions, Ordering::Release);
                    self.facts.generation.store(generation, Ordering::Release);
                    *reply
                } else {
                    reply
                };
                if let WorkerReply::InputDone {
                    legacy_reconnect, ..
                } = &reply
                {
                    self.facts
                        .legacy_reconnect
                        .store(*legacy_reconnect, Ordering::Release);
                }
                Ok(reply)
            }
            Ok(Ok(None)) | Err(RecvTimeoutError::Disconnected) => {
                Err(self.lost(io::Error::from(io::ErrorKind::UnexpectedEof)))
            }
            Ok(Err(error)) => Err(self.lost(error)),
            Err(RecvTimeoutError::Timeout) => {
                self.kill();
                let text = format!(
                    "the screen host process did not answer within {} s",
                    limit.as_secs()
                );
                Err(StageError::new(
                    ScreenErrorCode::CaptureFailed,
                    io::Error::new(io::ErrorKind::TimedOut, text),
                ))
            }
        }
    }

    /// The process is gone: its exit names the reason.
    fn lost(&mut self, error: io::Error) -> StageError {
        let status = self
            .child
            .as_mut()
            .and_then(|child| wait_for(child, EXIT_WAIT));
        if status.is_some() {
            self.broker.process_exited();
        }
        if status.is_none() {
            self.kill();
        }
        match status.and_then(|status| status.code()) {
            Some(crate::daemon::EXIT_UPDATE_STOP) => StageError::new(
                ScreenErrorCode::HostShutdown,
                io::Error::other("the screen host process ended for an update"),
            ),
            code => {
                let exit =
                    code.map_or_else(|| "no exit code".to_string(), |code| format!("exit {code}"));
                StageError::new(
                    ScreenErrorCode::CaptureFailed,
                    io::Error::other(format!(
                        "the screen host process ended unexpectedly ({exit}; {error})"
                    )),
                )
            }
        }
    }

    fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            if wait_for(child, EXIT_WAIT).is_some() {
                self.broker.process_exited();
            }
        }
    }
}

impl Drop for WorkerLink {
    fn drop(&mut self) {
        if let Ok(mut input) = self.input.lock() {
            let _ = send_request(&mut *input, &WorkerRequest::Close);
        }
        let status = self
            .child
            .as_mut()
            .and_then(|child| wait_for(child, EXIT_WAIT));
        if status.is_some() {
            self.broker.process_exited();
            self.child = None;
            return;
        }
        self.kill();
        if let Some(mut child) = self.child.take() {
            match child.try_wait() {
                Ok(Some(_)) => self.broker.process_exited(),
                _ => capture_lifetime::retire(child, self.broker.clone()),
            }
        }
    }
}

/// The link of a source, shared with the input path of its backend
/// (SCR-M6): input waits for a running picture request and goes next.
pub(super) type SharedLink = Arc<Mutex<WorkerLink>>;

#[path = "host_worker_source.rs"]
mod source;
pub(crate) use source::WorkerSource;
#[path = "host_worker_prepared.rs"]
mod prepared;

/// Where a host process for `source` comes from: the desktop program, or
/// for the test picture without it this program. `None` without either.
fn program_for<'a>(
    gui: Option<&'a Path>,
    current: Option<&'a Path>,
    source: &SourceId,
) -> Option<&'a Path> {
    gui.or(current.filter(|_| !source.is_screen_content()))
}

/// The desktop program of this installation, next to this program.
fn desktop_program() -> (Option<PathBuf>, Option<PathBuf>) {
    let current = std::env::current_exe().ok();
    let gui = current
        .as_deref()
        .and_then(|current| crate::share::consent_launch::gui_program(current, Path::is_file));
    (gui, current)
}

#[cfg(test)]
#[path = "host_worker_tests.rs"]
pub(crate) mod tests;
