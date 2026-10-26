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
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::gop_stream::now_us;
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SourceFrame,
};
use crate::screen::core::host_stage::{StageEncoder, StageFrame, StageRequest};
use crate::screen::core::negotiate::CodecChoice;
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
    book: usize,
}

struct Worker {
    admitted: Option<Option<crate::screen::core::frame_budget::FrameContract>>,
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
    reference_generation: u64,
    family: Option<super::host_worker_client::family_backend::WorkerFamily>,
    prepared: Option<Box<dyn crate::screen::core::capture_source_contract::PreparedSource>>,
    planned: Option<crate::screen::core::capture_admission::CapturePlan>,
    capture_session: Option<Arc<crate::screen::core::capture_admission::SessionAdmission>>,
    remote: Arc<dispatch::RemoteAuthority>,
    release_sink: Option<Arc<dyn crate::screen::core::host_backend::ScreenInputSink>>,
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
    run::run()
}

#[path = "capture_dispatch.rs"]
mod dispatch;
#[path = "host_worker_prepare.rs"]
mod prepare;
#[path = "host_worker_run.rs"]
mod run;

impl Worker {
    fn handle(&mut self, request: WorkerRequest) -> WorkerReply {
        match request {
            WorkerRequest::CaptureControl(request) => self.handle_capture_request(request),
            WorkerRequest::OpenFamily(_) | WorkerRequest::Open(_) => failed(
                ScreenErrorCode::CaptureFailed,
                "worker_open_requires_preparation_and_grant",
            ),
            WorkerRequest::Family(command) => {
                use crate::screen::core::worker_family::{FamilyCommand, FamilyReply};
                let result =
                    if self.remote.is_closed() && !matches!(&command, FamilyCommand::Snapshot) {
                        Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "family control cancelled by worker close",
                        ))
                    } else {
                        match (self.family.as_mut(), command) {
                            (Some(family), FamilyCommand::Snapshot) => family
                                .snapshot()
                                .map(|snapshot| FamilyReply::Snapshot(Some(snapshot))),
                            (None, FamilyCommand::Snapshot) => Ok(FamilyReply::Snapshot(None)),
                            (Some(family), FamilyCommand::Operation(request)) => {
                                family.operation(request).map(FamilyReply::Operation)
                            }
                            (Some(family), FamilyCommand::Input(request)) => {
                                family.input(request).map(FamilyReply::Input)
                            }
                            _ => Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "source_has_no_window_family",
                            )),
                        }
                    };
                match result {
                    Ok(reply) => WorkerReply::Family(reply),
                    Err(error) => capture_failed(&error),
                }
            }
            WorkerRequest::Release(events) => {
                let allowed = events.iter().all(|event| {
                    matches!(
                        event,
                        crate::screen::core::input_model::InputEvent::Key { down: false, .. }
                            | crate::screen::core::input_model::InputEvent::Button {
                                down: false,
                                ..
                            }
                            | crate::screen::core::input_model::InputEvent::Touch {
                                phase: crate::screen::core::input_model::TouchPhase::End
                                    | crate::screen::core::input_model::TouchPhase::Cancel,
                                ..
                            }
                    )
                });
                let blocked = if !allowed || !self.control {
                    Some(crate::screen::core::wire_control::ControlBlock::ControlUnavailable)
                } else if let Some(family) = self.family.as_mut() {
                    family.release().err()
                } else if let Some(sink) = self.release_sink.as_ref() {
                    sink.release(&events).err()
                } else {
                    self.backend.release_input(&events).err()
                };
                WorkerReply::InputDone {
                    blocked,
                    legacy_reconnect: self
                        .source
                        .as_ref()
                        .is_some_and(|s| s.legacy_input_invalid()),
                }
            }
            WorkerRequest::SetCaptureActive(active) => {
                self.capture_active = false;
                self.encoder = None;
                match self
                    .source
                    .as_mut()
                    .ok_or_else(|| io::Error::other("screen source unavailable"))
                    .and_then(|source| source.set_capture_active(active))
                {
                    Ok(()) => {
                        self.capture_active = active;
                        WorkerReply::CaptureActive(active)
                    }
                    Err(error) => capture_failed(&error),
                }
            }
            WorkerRequest::Produce(request) => self.produce(&request),
            WorkerRequest::Capture {
                frame_id,
                frame_contract,
            } => self.raw(frame_id, frame_contract),
            WorkerRequest::Input(input) => {
                let blocked = if !self.control
                    || self.source.is_none()
                    || self.family.is_some()
                    || self.remote.is_closed()
                {
                    Some(crate::screen::core::wire_control::ControlBlock::ControlUnavailable)
                } else {
                    self.source.as_mut().and_then(|source| {
                        source
                            .inject_input(
                                self.backend.as_ref(),
                                input.reference_generation,
                                input.presented,
                                &input.events,
                            )
                            .err()
                    })
                };
                WorkerReply::InputDone {
                    blocked,
                    legacy_reconnect: self
                        .source
                        .as_ref()
                        .is_some_and(|s| s.legacy_input_invalid()),
                }
            }
            WorkerRequest::Close => failed(ScreenErrorCode::CaptureFailed, "closed"),
        }
    }

    /// One raw picture for a still image.
    fn raw(
        &mut self,
        frame_id: u64,
        contract: Option<crate::screen::core::frame_budget::FrameContract>,
    ) -> WorkerReply {
        if let Err(error) = self.admit(contract) {
            return failed(ScreenErrorCode::CaptureFailed, error);
        }
        if !self.capture_active {
            return failed(ScreenErrorCode::CaptureFailed, "screen capture is paused");
        }
        let Some(source) = self.source.as_mut() else {
            return failed(ScreenErrorCode::CaptureFailed, "no source is open");
        };
        match source.capture_snapshot(frame_id, contract) {
            Ok(SourceFrame::Frame(image)) => WorkerReply::Raw(image),
            Ok(SourceFrame::Paused(state)) => WorkerReply::Frame(StageFrame::Paused(state)),
            Ok(SourceFrame::Unchanged) => {
                failed(ScreenErrorCode::CaptureFailed, "the source has no picture")
            }
            Err(error) => capture_failed(&error),
        }
    }

    /// RAW snapshots and encoded frames share one immutable session promise.
    fn admit(
        &mut self,
        contract: Option<crate::screen::core::frame_budget::FrameContract>,
    ) -> io::Result<()> {
        if self.remote.is_closed() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "worker capture cancelled by close",
            ));
        }
        if let Some(contract) = contract {
            contract.validate()?;
        }
        let Some(accepted) = self.admitted else {
            return Err(io::Error::other("worker capture contract is unbound"));
        };
        if self.capture_session.is_none()
            || self.source.is_none()
            || !crate::screen::core::frame_budget::contract_matches(contract, accepted)
        {
            return Err(io::Error::other(
                "worker request changed the admitted capture contract",
            ));
        }
        Ok(())
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

#[path = "host_worker_produce.rs"]
mod producing;
