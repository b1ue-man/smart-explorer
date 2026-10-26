//! The screen host process `--screen-host-worker` (SCR-M3), started by the
//! daemon per session (`host_worker_client`). It reads requests from its
//! standard input and answers on its standard output (frames of
//! `worker_wire`); nothing else may write there, so diagnostics go to
//! standard error. It opens the source with the platform backend, captures,
//! scales and encodes with the codecs of this user, opening a new encoder
//! whenever the daemon starts a new one (`encoder_epoch`), and drops a
//! running OpenH264 encoder once the user switched OpenH264 off (that
//! unloads the library). It ends on `CLOSE`, at the end of its input (the
//! daemon went), on a new update-stop request (exit 75, V8) and on Linux
//! with the daemon (parent-death signal). A source opened for a controlling
//! viewer (SCR-M6) takes `INPUT` and injects it with the platform backend;
//! any other session refuses input with `control_unavailable`.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::codec_inventory;
use super::openh264_switched_on;
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SourceFrame,
};
use crate::screen::core::host_stage::{StageEncoder, StageFrame, StageRequest};
use crate::screen::core::negotiate::CodecChoice;
use crate::screen::core::pixel_convert::downscale_bgra_with_limits;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::wire_hello::ScreenErrorCode;
use crate::screen::core::worker_wire::{read_message, send_reply, WorkerReply, WorkerRequest};

/// How often a running OpenH264 encoder looks whether it is still on.
const OPENH264_CHECK: Duration = Duration::from_secs(1);
/// Name of the OpenH264 encoder in its answers.
const OPENH264: &str = "openh264";
/// Test hook of the task suite (development builds only): the process
/// aborts after this many pictures, like a crashing codec.
#[cfg(debug_assertions)]
pub(crate) const CRASH_AFTER_ENV: &str = "SE_SCREEN_WORKER_CRASH_AFTER";

/// This process serves a session that may control (`OPEN` with `control`);
/// one process serves one session, so the flag is the process's.
static CONTROL: AtomicBool = AtomicBool::new(false);

/// Whether the session of this host process may control (the Wayland
/// backend asks the portal for `RemoteDesktop` then).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn session_controls() -> bool {
    CONTROL.load(Ordering::SeqCst)
}

/// The running encoder and what it was opened for.
struct Running {
    encoder: Box<dyn ScreenEncoder>,
    factory: String,
    hardware: bool,
    epoch: u64,
    choice: CodecChoice,
    frame_contract: Option<crate::screen::core::frame_budget::FrameContract>,
}

struct Worker {
    admitted:Option<Option<crate::screen::core::frame_budget::FrameContract>>,
    backend: Arc<dyn ScreenHostBackend>,
    source: Option<Box<dyn ScreenSource>>,
    encoder: Option<Running>,
    /// Factories whose encoder failed while encoding in this session.
    refused: Vec<String>,
    produced: u64,
    checked: Instant,
    /// The session may control (`OPEN` with `control`).
    control: bool,
    capture_active: bool,
    reference_generation:u64,
    family: Option<super::host_worker_client::family_backend::WorkerFamily>,
    memory:Option<crate::screen::core::frame_memory::MemoryLease>,
}

fn failed(code: ScreenErrorCode, error: impl ToString) -> WorkerReply {
    WorkerReply::Failed {
        code,
        message: error.to_string(),
    }
}

/// A capture error as the viewer sees it: a source the user declined
/// (`PermissionDenied`, the portal dialog of Linux hosts, SCR-M5) is
/// `no_display` with the reason in the text; everything else is
/// `capture_failed`.
fn capture_failed(error: &io::Error) -> WorkerReply {
    let code = if error.kind() == io::ErrorKind::PermissionDenied {
        ScreenErrorCode::NoDisplay
    } else {
        ScreenErrorCode::CaptureFailed
    };
    failed(code, error)
}

/// `--screen-host-worker` as the only argument (V9) runs the host process;
/// the result is its exit code. The entry of both programs
/// (`smart_explorer` in `run_gui`, `se` in its `main`).
pub fn run_screen_host_worker_if_requested(arguments: &[std::ffi::OsString]) -> Option<i32> {
    let requested = arguments.len() == 1
        && arguments[0]
            == std::ffi::OsStr::new(super::host_worker_client::SCREEN_HOST_WORKER_SWITCH);
    requested.then(run_host_worker)
}

/// Runs the host process; the exit code of `--screen-host-worker`.
pub(crate) fn run_host_worker() -> i32 {
    if let Err(error) = super::worker_lifetime::arm_worker() {
        eprintln!("screen host process: {error}");
        return 1;
    }
    let stop = crate::daemon::spawn_update_stop_watch(crate::daemon::UPDATE_STOP_POLL, || {
        std::process::exit(crate::daemon::EXIT_UPDATE_STOP)
    });
    let _stop = match stop {
        Ok(watch) => watch,
        Err(error) => {
            eprintln!("screen host process: update-stop watch: {error}");
            return 1;
        }
    };
    let mut worker = Worker {
        admitted:None,memory:None,
        backend: super::platform_backend(),
        source: None,
        encoder: None,
        refused: Vec::new(),
        produced: 0,
        checked: Instant::now(),
        control: false,
        capture_active: true,
        reference_generation:0,
        family:None,
    };
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        let (tag, payload) = match read_message(&mut input) {
            Ok(Some(message)) => message,
            Ok(None) => return 0,
            Err(error) => {
                eprintln!("screen host process: {error}");
                return 1;
            }
        };
        let request = match WorkerRequest::decode(tag, &payload) {
            Ok(WorkerRequest::Close) => return 0,
            Ok(request) => request,
            Err(error) => {
                eprintln!("screen host process: {error}");
                return 1;
            }
        };
        let reply = worker.handle(request);
        let generation=worker.source.as_ref().map_or(0,|source|source.reference_generation());
        if generation!=worker.reference_generation {worker.reference_generation=generation;worker.encoder=None;}
        let regions=worker.source.as_ref().is_none_or(|source|source.supports_frame_regions());
        let limits=worker.source.as_ref().map_or(FrameLimits::LEGACY,|source|source.frame_limits());
        let family_layout_revision=worker.source.as_ref().and_then(|source|source.family_layout_revision());
        let reply=WorkerReply::Reference {generation,regions,limits,family_layout_revision,reply:Box::new(reply)};
        if let Err(error) = send_reply(&mut output, &reply) {
            eprintln!("screen host process: {error}");
            return 1;
        }
    }
}

impl Worker {
    fn handle(&mut self, request: WorkerRequest) -> WorkerReply {
        match request {
            WorkerRequest::OpenFamily(open) => {
                self.family=None; self.source=None; self.encoder=None;
                CONTROL.store(open.control,Ordering::SeqCst);self.control=open.control;self.capture_active=true;
                match super::host_worker_client::family_backend::WorkerFamily::open(open,self.backend.clone()) {
                    Ok((family,source))=>{let size=source.size();self.reference_generation=source.reference_generation();
                        self.family=Some(family);self.source=Some(source);WorkerReply::Opened {size}},
                    Err(error)=>WorkerReply::Refused {reason:error.to_string()},
                }
            }
            WorkerRequest::Family(command) => {
                use crate::screen::core::worker_family::{FamilyCommand,FamilyReply};
                let result=match (self.family.as_mut(),command) {
                    (Some(family),FamilyCommand::Snapshot)=>family.snapshot().map(|snapshot|FamilyReply::Snapshot(Some(snapshot))),
                    (None,FamilyCommand::Snapshot)=>Ok(FamilyReply::Snapshot(None)),
                    (Some(family),FamilyCommand::Operation(request))=>family.operation(request).map(FamilyReply::Operation),
                    (Some(family),FamilyCommand::Input(request))=>family.input(request).map(FamilyReply::Input),
                    _=>Err(io::Error::new(io::ErrorKind::PermissionDenied,"source_has_no_window_family")),
                };
                match result {Ok(reply)=>WorkerReply::Family(reply),Err(error)=>capture_failed(&error)}
            }
            WorkerRequest::Release(events) => {
                let allowed=events.iter().all(|event|matches!(event,
                    crate::screen::core::input_model::InputEvent::Key {down:false,..}
                    |crate::screen::core::input_model::InputEvent::Button {down:false,..}
                    |crate::screen::core::input_model::InputEvent::Touch {phase:crate::screen::core::input_model::TouchPhase::End|crate::screen::core::input_model::TouchPhase::Cancel,..}));
                let blocked=if !allowed || !self.control {Some(crate::screen::core::wire_control::ControlBlock::ControlUnavailable)}
                    else if let Some(family)=self.family.as_mut() {family.release().err()} else {self.backend.release_input(&events).err()};
                WorkerReply::InputDone {blocked,legacy_reconnect:self.source.as_ref().is_some_and(|s|s.legacy_input_invalid())}
            }
            WorkerRequest::Open(open) => {
                self.family=None;self.source=None;
                self.encoder = None;
                CONTROL.store(open.control, Ordering::SeqCst);
                self.control = open.control;
                self.capture_active=true;
                match self.backend.open_source(&open.source, open.wanted) {
                    Ok(source) => {
                        let size = source.size();
                        self.reference_generation=source.reference_generation();
                        super::note_input_target(&open.source);
                        self.source = Some(source);
                        WorkerReply::Opened { size }
                    }
                    Err(refusal) => WorkerReply::Refused {
                        reason: refusal.reason,
                    },
                }
            }
            WorkerRequest::SetCaptureActive(active) => {
                self.capture_active=false; self.encoder=None;
                match self.source.as_mut().ok_or_else(||io::Error::other("screen source unavailable")).and_then(|source|source.set_capture_active(active)) {
                    Ok(()) => {self.capture_active=active; WorkerReply::CaptureActive(active)},
                    Err(error) => capture_failed(&error),
                }
            }
            WorkerRequest::Produce(request) => self.produce(&request),
            WorkerRequest::Capture { frame_id,frame_contract } => self.raw(frame_id,frame_contract),
            WorkerRequest::Input(input) => {
                let blocked = if !self.control || self.source.is_none() || self.family.is_some() {
                    Some(crate::screen::core::wire_control::ControlBlock::ControlUnavailable)
                } else {
                    self.source.as_mut().and_then(|source|source.inject_input(self.backend.as_ref(),
                        input.reference_generation,input.presented,&input.events).err())
                };
                WorkerReply::InputDone { blocked,legacy_reconnect:self.source.as_ref().is_some_and(|s|s.legacy_input_invalid()) }
            }
            WorkerRequest::Close => failed(ScreenErrorCode::CaptureFailed, "closed"),
        }
    }

    /// One raw picture for a still image.
    fn raw(&mut self, frame_id: u64,contract:Option<crate::screen::core::frame_budget::FrameContract>) -> WorkerReply {
        if let Err(error)=self.admit(contract) {return failed(ScreenErrorCode::CaptureFailed,error);}
        if !self.capture_active {return failed(ScreenErrorCode::CaptureFailed,"screen capture is paused");}
        let Some(source) = self.source.as_mut() else {
            return failed(ScreenErrorCode::CaptureFailed, "no source is open");
        };
        match source.capture_snapshot(frame_id,contract) {
            Ok(SourceFrame::Frame(image)) => WorkerReply::Raw(image),
            Ok(SourceFrame::Paused(state)) => WorkerReply::Frame(StageFrame::Paused(state)),
            Ok(SourceFrame::Unchanged) => {
                failed(ScreenErrorCode::CaptureFailed, "the source has no picture")
            }
            Err(error) => capture_failed(&error),
        }
    }

    /// RAW snapshots and encoded frames share one immutable session promise.
    fn admit(&mut self,contract:Option<crate::screen::core::frame_budget::FrameContract>)->io::Result<()> {
        if let Some(contract)=contract {contract.validate()?;}
        if let Some(previous)=self.admitted {
            let same=match (contract,previous) {
                (None,None)=>true,(Some(a),Some(b))=>a.version==b.version && a.limits==b.limits,_=>false,
            };
            if !same {return Err(io::Error::other("worker request changed admitted frame contract"));}
        } else {
            let bytes=contract.map_or(crate::screen::core::frame_budget::MAX_WORKING_BYTES,|contract|contract.limits.working_bytes);
            let lease=crate::screen::core::frame_memory::reserve(self.backend.memory_facts(),bytes)?;
            if lease.bytes()!=bytes {return Err(io::Error::other("worker cannot honor session memory promise"));}
            self.memory=Some(lease);self.admitted=Some(contract);
        }
        Ok(())
    }
    fn produce(&mut self, request: &StageRequest) -> WorkerReply {
        if let Err(error)=self.admit(request.frame_contract) {return failed(ScreenErrorCode::CaptureFailed,error);}
        if !self.capture_active { return self.source.as_ref().map_or_else(||failed(ScreenErrorCode::CaptureFailed,"screen source unavailable"),|source|WorkerReply::Frame(StageFrame::Unchanged {source_size:source.size()})); }
        self.crash_hook();
        self.check_openh264();
        let refresh = request.keyframe
            || self.encoder.as_ref().is_none_or(|running| {
                running.epoch != request.encoder_epoch
                    || running.choice != request.choice
                    || running.frame_contract != request.frame_contract
                    || running.encoder.size() != request.size
                    || running.encoder.needs_refresh()
            });
        let Some(source) = self.source.as_mut() else {
            return failed(ScreenErrorCode::CaptureFailed, "no source is open");
        };
        let source_size = source.size();
        let limits=request.frame_contract.map_or(FrameLimits::LEGACY,|contract|contract.limits)
            .for_capture(source.frame_limits());
        if let Err(error)=limits.check(request.size) {return failed(ScreenErrorCode::CaptureFailed,error);}
        let before=source.reference_generation();
        if before!=self.reference_generation {
            self.reference_generation=before;self.encoder=None;
            return WorkerReply::Frame(StageFrame::Unchanged {source_size});
        }
        // A request made for an older, larger source waits for the next.
        if request.size.width > source_size.width || request.size.height > source_size.height {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        let capture_ts_us = now_us();
        let captured=source.capture(request.frame_id,refresh);
        let after=source.reference_generation();let source_size=source.size();
        if after!=before {
            self.reference_generation=after;self.encoder=None;
            return match captured {
                Ok(SourceFrame::Paused(state))=>WorkerReply::Frame(StageFrame::Paused(state)),
                Err(error)=>capture_failed(&error),
                _=>WorkerReply::Frame(StageFrame::Unchanged {source_size}),
            };
        }
        let image = match captured {
            Ok(SourceFrame::Frame(image)) => image,
            Ok(SourceFrame::Unchanged) => {
                let source_size = source.size();
                return WorkerReply::Frame(StageFrame::Unchanged { source_size });
            }
            Ok(SourceFrame::Paused(state)) => return WorkerReply::Frame(StageFrame::Paused(state)),
            Err(error) => return capture_failed(&error),
        };
        let source_size = image.size();
        if request.size.width > source_size.width || request.size.height > source_size.height {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        let image = if image.size() == request.size {
            image
        } else {
            match downscale_bgra_with_limits(&image.view(), request.size, limits) {
                Ok(image) => image,
                Err(error) => return failed(ScreenErrorCode::CaptureFailed, error),
            }
        };
        // An encoder that fails while encoding gives way to the next one of
        // the same codec (a hardware MFT to the software MFT, C2) once.
        let mut attempt = 0;
        let (frame, running) = loop {
            attempt += 1;
            let running = match self.encoder_for(request) {
                Ok(running) => running,
                Err(error) => return failed(ScreenErrorCode::EncoderFailed, error),
            };
            match running
                .encoder
                .encode_frame(&image, request)
            {
                Ok(frame) => break (frame, running),
                Err(error) => {
                    let factory = running.factory.clone();
                    self.encoder = None;
                    if attempt > 1 || factory == crate::screen::core::codec_registry::TILES_NAME {
                        return failed(ScreenErrorCode::EncoderFailed, error);
                    }
                    self.refused.push(factory);
                }
            }
        };
        if frame.records.is_empty() {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        if let Err(error) = crate::screen::core::frame_size_caps::validate_encoded_budget(
            request.frame_contract, request.choice.codec, &frame.records,
        ) {
            return failed(ScreenErrorCode::EncoderFailed, error);
        }
        let encoder = StageEncoder {
            name: running.encoder.name(),
            hardware: running.hardware,
        };
        WorkerReply::Frame(StageFrame::Encoded {
            records: frame.records,
            keyframe: frame.keyframe,
            capture_ts_us,
            encoder,
            source_size,
        })
    }

    /// The encoder for `request`: the running one while epoch, codec and
    /// size stay, else a new one from the codecs of this user (the old one
    /// is dropped first, so a library it held is unloaded before the next
    /// loads).
    fn encoder_for(&mut self, request: &StageRequest) -> io::Result<&mut Running> {
        let current = self.encoder.as_ref().is_some_and(|running| {
            running.epoch == request.encoder_epoch
                && running.choice == request.choice
                && running.frame_contract == request.frame_contract
                && running.encoder.size() == request.size
        });
        if !current {
            self.encoder = None;
            let opened = codec_inventory::registry()
                .open_encoder_for_stage(request, &self.refused)?;
            self.encoder = Some(Running {
                encoder: opened.encoder,
                factory: opened.factory,
                hardware: opened.hardware,
                epoch: request.encoder_epoch,
                choice: request.choice,
                frame_contract: request.frame_contract,
            });
        }
        self.encoder
            .as_mut()
            .ok_or_else(|| io::Error::other("no screen encoder"))
    }

    /// Once a second: a running OpenH264 encoder goes when the user switched
    /// OpenH264 off; the next picture opens another encoder or fails over to
    /// the tiles in the daemon.
    fn check_openh264(&mut self) {
        if self.checked.elapsed() < OPENH264_CHECK {
            return;
        }
        self.checked = Instant::now();
        let openh264 = self
            .encoder
            .as_ref()
            .is_some_and(|running| running.encoder.name() == OPENH264);
        if openh264 && !openh264_switched_on() {
            self.encoder = None;
        }
    }

    #[cfg(debug_assertions)]
    fn crash_hook(&mut self) {
        self.produced += 1;
        let limit = std::env::var(CRASH_AFTER_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok());
        if limit.is_some_and(|limit| self.produced > limit) {
            std::process::abort();
        }
    }

    #[cfg(not(debug_assertions))]
    fn crash_hook(&mut self) {
        self.produced += 1;
    }
}

#[cfg(test)]
#[path = "host_worker_main_tests.rs"]
mod tests;
