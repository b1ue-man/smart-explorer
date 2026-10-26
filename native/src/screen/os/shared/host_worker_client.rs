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
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::sync::atomic::{AtomicBool,AtomicU64,Ordering};
use std::time::{Duration, Instant};

use crate::screen::core::host_backend::{ScreenSource, SourceFrame, SourceRefusal};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_stage::{HostStage, StageError, StageFrame, StageRequest};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_hello::ScreenErrorCode;
use crate::screen::core::worker_wire::{
    send_request, OpenRequest, WorkerReply, WorkerRequest,
};

#[path = "host_worker_reader.rs"]
mod reader;
use reader::WorkerReader;
#[path = "host_worker_backend.rs"]
mod backend;
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

struct SourceFacts {generation:AtomicU64,regions:AtomicBool,limits:Mutex<FrameLimits>,family_layout_revision:AtomicU64,legacy_reconnect:AtomicBool}
impl Default for SourceFacts {fn default()->Self {Self {generation:AtomicU64::new(0),regions:AtomicBool::new(true),limits:Mutex::new(FrameLimits::LEGACY),family_layout_revision:AtomicU64::new(0),legacy_reconnect:AtomicBool::new(false)}}}

/// One running host process.
pub(super) struct WorkerLink {
    facts:Arc<SourceFacts>,
    child: Child,
    input: ChildStdin,
    reader: WorkerReader,
}

impl WorkerLink {
    fn start(program: &Path, env: &[(String, String)]) -> io::Result<Self> {
        let mut command = Command::new(program);
        command.envs(env.iter().map(|(key, value)| (key, value)));
        command
            .arg(SCREEN_HOST_WORKER_SWITCH)
            .env(PARENT_ENV, std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        super::worker_lifetime::prepare(&mut command);
        Self::with_command(command)
    }

    fn with_command(command: Command) -> io::Result<Self> {
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
        let reader = match WorkerReader::start(output) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            facts:Arc::new(SourceFacts::default()),
            child,
            input,
            reader,
        })
    }

    /// Sends `request` and waits up to `limit` for the answer.
    pub(super) fn ask(
        &mut self,
        request: &WorkerRequest,
        limit: Duration,
    ) -> Result<WorkerReply, StageError> {
        let limits = match request {
            WorkerRequest::Produce(stage) => stage.frame_contract.map(|contract| contract.limits),
            _ => None,
        };
        if let Err(error) = self.reader.begin(limits) {
            return Err(self.lost(error));
        }
        if let Err(error) = send_request(&mut self.input, request) {
            return Err(self.lost(error));
        }
        match self.reader.receive(limit) {
            Ok(Ok(Some((tag, payload)))) => {
                let reply=WorkerReply::decode(tag,&payload).map_err(|error| {self.kill();StageError::new(ScreenErrorCode::CaptureFailed,error)})?;
                let reply=if let WorkerReply::Reference {generation,regions,limits,family_layout_revision,reply}=reply {
                    if generation<self.facts.generation.load(Ordering::Acquire) {self.kill();return Err(StageError::new(ScreenErrorCode::CaptureFailed,io::Error::other("worker source generation regressed")));}
                    *self.facts.limits.lock().unwrap_or_else(PoisonError::into_inner)=limits;
                    self.facts.family_layout_revision.store(family_layout_revision.unwrap_or(0),Ordering::Release);
                    self.facts.regions.store(regions,Ordering::Release);self.facts.generation.store(generation,Ordering::Release);*reply
                } else {reply};
                if let WorkerReply::InputDone {legacy_reconnect,..}=&reply {self.facts.legacy_reconnect.store(*legacy_reconnect,Ordering::Release);}
                Ok(reply)
            },
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
        let status = wait_for(&mut self.child, EXIT_WAIT);
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
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for WorkerLink {
    fn drop(&mut self) {
        let _ = send_request(&mut self.input, &WorkerRequest::Close);
        if wait_for(&mut self.child, EXIT_WAIT).is_none() {
            self.kill();
        }
        // The reader ends with the process's output; it is not joined, so
        // a full answer slot cannot hold this drop.
    }
}

/// The link of a source, shared with the input path of its backend
/// (SCR-M6): input waits for a running picture request and goes next.
pub(super) type SharedLink = Arc<Mutex<WorkerLink>>;

/// A source captured and encoded in its host process.
pub(crate) struct WorkerSource {
    facts:Arc<SourceFacts>,
    link: SharedLink,
    size: FrameSize,
}
struct WorkerInputSink(SharedLink,Arc<SourceFacts>);
impl WorkerInputSink {
    fn family(&self, command: crate::screen::core::worker_family::FamilyCommand, active:Option<Arc<AtomicBool>>)
        -> io::Result<crate::screen::core::worker_family::FamilyReply> {
        let mut link=self.0.lock().map_err(|_|io::Error::other("family_worker_link_poisoned"))?;
        if active.is_some_and(|active|!active.load(Ordering::Acquire)) {
            return Err(io::Error::new(io::ErrorKind::Interrupted,"family_request_cancelled"));
        }
        match link.ask(&WorkerRequest::Family(command),INPUT_TIMEOUT).map_err(|failure|failure.error)? {
            WorkerReply::Family(reply)=>Ok(reply),
            WorkerReply::Failed {message,..}=>Err(io::Error::other(message)),
            _=>Err(io::Error::other("unexpected family worker response")),
        }
    }
}
impl crate::screen::core::host_backend::ScreenInputSink for WorkerInputSink {
    fn legacy_input_invalid(&self)->bool {self.1.legacy_reconnect.load(Ordering::Acquire)}
    fn inject(&self,events:&[crate::screen::core::input_model::InputEvent],reference:u64,presented:bool)
        ->Result<(),crate::screen::core::wire_control::ControlBlock> {
        backend::send_bound_input(&self.0,events,Some(reference),presented)
    }
    fn family_snapshot(&self)->io::Result<Option<crate::remote_window::FamilySnapshot>> {
        match self.family(crate::screen::core::worker_family::FamilyCommand::Snapshot,None)? {
            crate::screen::core::worker_family::FamilyReply::Snapshot(snapshot)=>Ok(snapshot),
            _=>Err(io::Error::other("unexpected family snapshot response")),
        }
    }
    fn family_operation(&self,request:crate::screen::core::family_protocol::FamilyOperation,active:Arc<AtomicBool>)
        ->io::Result<crate::screen::core::family_protocol::FamilyOperationResult> {
        match self.family(crate::screen::core::worker_family::FamilyCommand::Operation(request),Some(active))? {
            crate::screen::core::worker_family::FamilyReply::Operation(result)=>Ok(result),
            _=>Err(io::Error::other("unexpected family operation response")),
        }
    }
    fn family_input(&self,request:crate::screen::core::family_protocol::FamilyInput,active:Arc<AtomicBool>)
        ->io::Result<crate::screen::core::family_protocol::FamilyInputResult> {
        match self.family(crate::screen::core::worker_family::FamilyCommand::Input(request),Some(active))? {
            crate::screen::core::worker_family::FamilyReply::Input(result)=>Ok(result),
            _=>Err(io::Error::other("unexpected family input response")),
        }
    }
    fn release(&self,events:&[crate::screen::core::input_model::InputEvent])->Result<(),crate::screen::core::wire_control::ControlBlock> {
        use crate::screen::core::wire_control::ControlBlock;
        // The picture request already has FRAME_TIMEOUT; retain this exact worker
        // until its admitted capture completes, then release its pressed inputs.
        let mut link=self.0.lock().map_err(|_|ControlBlock::ControlUnavailable)?;
        match link.ask(&WorkerRequest::Release(events.to_vec()),INPUT_TIMEOUT) {
            Ok(WorkerReply::InputDone {blocked:None,..})=>Ok(()),
            Ok(WorkerReply::InputDone {blocked:Some(blocked),..})=>Err(blocked),
            _=>Err(ControlBlock::ControlUnavailable),
        }
    }
}

impl WorkerSource {
    pub(crate) fn open_family(program: &Path, env: &[(String,String)],
        open: crate::screen::core::worker_family::OpenFamily) -> Result<Self, SourceRefusal> {
        let mut link = WorkerLink::start(program, env)
            .map_err(|error| SourceRefusal::new(&format!("host_process: {error}")))?;
        match link.ask(&WorkerRequest::OpenFamily(open), OPEN_TIMEOUT) {
            Ok(WorkerReply::Opened {size}) => Ok(Self { facts: link.facts.clone(),
                link: Arc::new(Mutex::new(link)), size }),
            Ok(WorkerReply::Refused {reason}) => Err(SourceRefusal::new(&reason)),
            Ok(_) => Err(SourceRefusal::new("unexpected OPEN_FAMILY response")),
            Err(error) => Err(SourceRefusal::new(&error.error.to_string())),
        }
    }
    /// Starts a host process with `program` (and `env` beside the daemon's
    /// environment) and opens `source` there.
    /// `control`: the session may control, so the process prepares input.
    pub(crate) fn open(
        program: &Path,
        env: &[(String, String)],
        source: &SourceId,
        wanted: Option<FrameSize>,
        control: bool,
    ) -> Result<Self, SourceRefusal> {
        let refusal = |error: io::Error| SourceRefusal::new(&format!("host_process: {error}"));
        let link = WorkerLink::start(program, env).map_err(refusal)?;
        Self::opened(link, source, wanted, control)
    }

    fn opened(
        mut link: WorkerLink,
        source: &SourceId,
        wanted: Option<FrameSize>,
        control: bool,
    ) -> Result<Self, SourceRefusal> {
        let request = WorkerRequest::Open(OpenRequest {
            source: source.clone(),
            wanted,
            control,
        });
        match link.ask(&request, OPEN_TIMEOUT) {
            Ok(WorkerReply::Opened { size }) => Ok(Self {

                facts:link.facts.clone(),
                link: Arc::new(Mutex::new(link)),
                size,
            }),
            Ok(WorkerReply::Refused { reason }) => Err(SourceRefusal::new(&reason)),
            Ok(_) => Err(SourceRefusal::new(
                "host_process: unexpected answer to OPEN",
            )),
            Err(failure) => Err(SourceRefusal::new(&format!(
                "host_process: {}",
                failure.error
            ))),
        }
    }

    /// The link, for the input path of the backend.
    pub(super) fn shared_link(&self) -> SharedLink {
        self.link.clone()
    }

    fn link(&self) -> MutexGuard<'_, WorkerLink> {
        self.link.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn unexpected(&mut self, what: &str) -> StageError {
        self.link().kill();
        StageError::new(
            ScreenErrorCode::CaptureFailed,
            io::Error::other(format!(
                "unexpected answer of the screen host process to {what}"
            )),
        )
    }
}

impl ScreenSource for WorkerSource {
    fn family_layout_revision(&self)->Option<u64> {let revision=self.facts.family_layout_revision.load(Ordering::Acquire);(revision!=0).then_some(revision)}
    fn frame_limits(&self)->FrameLimits {*self.facts.limits.lock().unwrap_or_else(PoisonError::into_inner)}
    fn input_sink(&self)->Option<Arc<dyn crate::screen::core::host_backend::ScreenInputSink>> {Some(Arc::new(WorkerInputSink(self.link.clone(),self.facts.clone())))}
    fn reference_generation(&self)->u64 {self.facts.generation.load(Ordering::Acquire)}
    fn supports_frame_regions(&self)->bool {self.facts.regions.load(Ordering::Acquire)}
    fn size(&self) -> FrameSize {
        self.size
    }

    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        match self.link().ask(&WorkerRequest::SetCaptureActive(active),FRAME_TIMEOUT) {
            Ok(WorkerReply::CaptureActive(value)) if value==active => Ok(()),
            Ok(WorkerReply::Failed {message,..}) => Err(io::Error::other(message)),
            Ok(_) => Err(io::Error::other("unexpected host capture activity acknowledgement")),
            Err(failure) => Err(failure.error),
        }
    }

    /// One raw picture (still images); video goes through `host_stage`.
    fn capture(&mut self, frame_id: u64, _keyframe: bool) -> io::Result<SourceFrame> {
        let answer = self
            .link()
            .ask(&WorkerRequest::Capture { frame_id }, FRAME_TIMEOUT);
        match answer {
            Ok(WorkerReply::Raw(image)) => {self.size=image.size();Ok(SourceFrame::Frame(image))},
            Ok(WorkerReply::Frame(StageFrame::Paused(state))) => Ok(SourceFrame::Paused(state)),
            Ok(WorkerReply::Failed { message, .. }) => Err(io::Error::other(message)),
            Ok(_) => Err(self.unexpected("CAPTURE").error),
            Err(failure) => Err(failure.error),
        }
    }

    fn host_stage(&mut self) -> Option<&mut dyn HostStage> {
        Some(self)
    }
}

impl HostStage for WorkerSource {
    fn produce(&mut self, request: &StageRequest) -> Result<StageFrame, StageError> {
        let answer = self
            .link()
            .ask(&WorkerRequest::Produce(request.clone()), FRAME_TIMEOUT)?;
        match answer {
            WorkerReply::Frame(frame) => {
                match &frame {
                    StageFrame::Encoded { source_size, .. }
                    | StageFrame::Unchanged { source_size } => self.size = *source_size,
                    StageFrame::Paused(_) => {}
                }
                Ok(frame)
            }
            WorkerReply::Failed { code, message } => {
                Err(StageError::new(code, io::Error::other(message)))
            }
            _ => Err(self.unexpected("PRODUCE")),
        }
    }
}

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
