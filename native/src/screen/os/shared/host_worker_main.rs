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
//! with the daemon (parent-death signal).
use std::io;
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
use crate::screen::core::pixel_convert::downscale_bgra;
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

/// The running encoder and what it was opened for.
struct Running {
    encoder: Box<dyn ScreenEncoder>,
    factory: String,
    hardware: bool,
    epoch: u64,
    choice: CodecChoice,
}

struct Worker {
    backend: Arc<dyn ScreenHostBackend>,
    source: Option<Box<dyn ScreenSource>>,
    encoder: Option<Running>,
    /// Factories whose encoder failed while encoding in this session.
    refused: Vec<String>,
    produced: u64,
    checked: Instant,
}

fn failed(code: ScreenErrorCode, error: impl ToString) -> WorkerReply {
    WorkerReply::Failed {
        code,
        message: error.to_string(),
    }
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
        backend: super::platform_backend(),
        source: None,
        encoder: None,
        refused: Vec::new(),
        produced: 0,
        checked: Instant::now(),
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
        if let Err(error) = send_reply(&mut output, &reply) {
            eprintln!("screen host process: {error}");
            return 1;
        }
    }
}

impl Worker {
    fn handle(&mut self, request: WorkerRequest) -> WorkerReply {
        match request {
            WorkerRequest::Open(open) => {
                self.encoder = None;
                match self.backend.open_source(&open.source, open.wanted) {
                    Ok(source) => {
                        let size = source.size();
                        self.source = Some(source);
                        WorkerReply::Opened { size }
                    }
                    Err(refusal) => WorkerReply::Refused {
                        reason: refusal.reason,
                    },
                }
            }
            WorkerRequest::Produce(request) => self.produce(&request),
            WorkerRequest::Capture { frame_id } => self.raw(frame_id),
            WorkerRequest::Close => failed(ScreenErrorCode::CaptureFailed, "closed"),
        }
    }

    /// One raw picture for a still image.
    fn raw(&mut self, frame_id: u64) -> WorkerReply {
        let Some(source) = self.source.as_mut() else {
            return failed(ScreenErrorCode::CaptureFailed, "no source is open");
        };
        match source.capture(frame_id, true) {
            Ok(SourceFrame::Frame(image)) => WorkerReply::Raw(image),
            Ok(SourceFrame::Paused(state)) => WorkerReply::Frame(StageFrame::Paused(state)),
            Ok(SourceFrame::Unchanged) => {
                failed(ScreenErrorCode::CaptureFailed, "the source has no picture")
            }
            Err(error) => failed(ScreenErrorCode::CaptureFailed, error),
        }
    }

    fn produce(&mut self, request: &StageRequest) -> WorkerReply {
        self.crash_hook();
        self.check_openh264();
        let Some(source) = self.source.as_mut() else {
            return failed(ScreenErrorCode::CaptureFailed, "no source is open");
        };
        let source_size = source.size();
        // A request made for an older, larger source waits for the next.
        if request.size.width > source_size.width || request.size.height > source_size.height {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        let capture_ts_us = now_us();
        let image = match source.capture(request.frame_id, request.keyframe) {
            Ok(SourceFrame::Frame(image)) => image,
            Ok(SourceFrame::Unchanged) => {
                let source_size = source.size();
                return WorkerReply::Frame(StageFrame::Unchanged { source_size });
            }
            Ok(SourceFrame::Paused(state)) => return WorkerReply::Frame(StageFrame::Paused(state)),
            Err(error) => return failed(ScreenErrorCode::CaptureFailed, error),
        };
        let source_size = image.size();
        let image = if image.size() == request.size {
            image
        } else {
            match downscale_bgra(&image.view(), request.size) {
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
                .encode(&image, request.keyframe, request.target_bps)
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
                && running.encoder.size() == request.size
        });
        if !current {
            self.encoder = None;
            let opened = codec_inventory::registry().open_encoder_except(
                &request.choice,
                request.size,
                request.fps,
                &self.refused,
            )?;
            self.encoder = Some(Running {
                encoder: opened.encoder,
                factory: opened.factory,
                hardware: opened.hardware,
                epoch: request.encoder_epoch,
                choice: request.choice,
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
