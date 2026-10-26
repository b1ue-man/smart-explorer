//! The picture side of one host session (decisions C2, C3, C6): the frame
//! clock takes captured frames at the wanted rate; the pacer decides before
//! capture and encoding whether a frame fits the queue guard (`Pacer::decide`
//! with the path and round trip of the connection), resets a stale picture
//! stream and asks for key frames; capture and encoding run on the blocking
//! pool, in the host process when the source was opened there (SCR-M3,
//! `ScreenSource::host_stage`); the records go to the picture stream of the
//! current picture group, stamped after the last one (`SentFrame.sent_us`).
//! `FRAME_ACK` feeds pacer and rate control, the cumulative loss counters
//! arrive once a second. A picture stream that does not take a frame within
//! the stale time, or fails, is reset and the next frame is a key frame on a
//! new stream (`Pacer::on_stream_reset`). An H.264 encoder that fails gives
//! way to the lossless tile codec, the last fallback every viewer decodes.
//! All sessions to a peer share sender credit before capture/encode.
//! A large lossless reference streams within its serialization budget. Previews travel one at a
//! time: the next one only after `PREVIEW_ACK` and the chosen interval.
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;

use super::gop_stream::{now_us, path_of, GopWriter};
use super::host_backend::{CapturedEncodedFrame as EncodedFrame, ScreenHostBackend, ScreenSource};
use super::host_stage::{StageEncoder, StageError, StageRequest};
use super::negotiate::{CodecChoice, CodecKind};
use super::pacing::{FrameClock, PaceDecision, Pacer, SentFrame, STALE_STREAM_US};
use super::pixel_convert::FrameSize;
use super::rate_control::{Quality, RateController};
use super::session_state::SessionState;
use super::wire_binary::{GopHeader, StreamKind};
use super::wire_control::StreamInfo;
use super::wire_hello::ScreenErrorCode;

#[path = "host_capture.rs"]
mod capture;
use capture::{produce, Captured, Stage, StageOutput};
#[path = "host_mode.rs"]
mod mode;
#[path = "host_quality.rs"]
mod quality;
use quality::{QualityMode, QualityPolicy};
#[path = "host_frame_send.rs"]
mod frame_send;
#[path = "host_capture_pause.rs"]
mod capture_pause;
#[path = "host_reference.rs"]
mod reference;
use reference::ReferencePresentation;
#[path = "host_work_control.rs"]
mod work_control;
#[path = "host_pipeline_actor.rs"]
mod actor;
pub(crate) use actor::{PipelineActor,WorkCompletion};

/// Long edge of a tile preview unless the group view asks for more (C6).
pub(crate) const PREVIEW_EDGE: u32 = 320;
/// Interval between two previews unless the group view asks otherwise.
pub(crate) const PREVIEW_INTERVAL_US: u64 = 1_000_000;

/// What one capture tick produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Produced {
    /// Not taken (frame clock, preview interval) or nothing changed.
    Idle,
    /// Skipped before encoding (queue guard).
    Dropped,
    Sent,
    /// The picture stream was given up; the next frame is a key frame.
    Reset,
    /// The source pauses (secure desktop, minimized window, …).
    Paused(SessionState),
    /// The connection is closed: the session ends with it.
    Closed,
}

/// A failure of the picture side that ends the session.
#[derive(Debug)]
pub(crate) struct PipelineError {
    pub code: ScreenErrorCode,
    pub error: io::Error,
}

impl PipelineError {
    fn capture(error: io::Error) -> Self {
        Self {
            code: ScreenErrorCode::CaptureFailed,
            error,
        }
    }

    fn encoder(error: io::Error) -> Self {
        Self {
            code: ScreenErrorCode::EncoderFailed,
            error,
        }
    }
}

impl From<StageError> for PipelineError {
    fn from(failure: StageError) -> Self {
        Self {
            code: failure.code,
            error: failure.error,
        }
    }
}

/// Counters of one session for the statistics and the access log.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Totals {
    pub bytes: u64,
    pub frames: u64,
    pub keyframes: u64,
}

pub(crate) struct Pipeline {
    work:Arc<work_control::WorkControl>,
    backend: Arc<dyn ScreenHostBackend>,
    frame_contract: Option<super::frame_budget::FrameContract>,
    negotiated_contract: Option<super::frame_budget::FrameContract>,
    sender: super::sender_budget::SenderBudget,
    route_epoch: Option<(u64,u64,Option<crate::share::route_state::RouteClass>)>,
    stage: Option<Stage>,
    source_size: FrameSize,
    source_generation:u64,
    presentation:ReferencePresentation,
    capture_active: bool,
    capture_applied: Option<bool>,
    capture_requested: bool,
    capture_ready: Option<io::Result<()>>,
    capture_transition: Option<tokio::task::JoinHandle<(Stage,bool,io::Result<()>)>>,
    visible_rects: Option<Vec<super::visibility::VisibleRect>>,
    choice: CodecChoice,
    quality_policy: QualityPolicy,
    fallback_notice: Option<String>,
    tile_limit: Option<FrameSize>,
    kind: StreamKind,
    gop: Option<GopWriter>,
    next_gop_id: u64,
    next_frame_id: u64,
    pacer: Pacer,
    rate: RateController,
    clock: FrameClock,
    codec_fps:u32,
    quality: Quality,
    max_fps: u32,
    /// The viewer's display from its hello.
    display: FrameSize,
    /// The largest picture of the current mode.
    bounds: FrameSize,
    preview_interval_us: u64,
    preview_in_flight: Option<u64>,
    preview_sent_us: u64,
    totals: Totals,
    second_bytes: u64,
    second_frames: u32,
    /// Test hook: picture streams to reset after the next frames.
    inject_resets: Arc<AtomicU32>,
    /// Counts the encoders started; a host process opens a new one with it.
    encoder_epoch: u64,
    /// The running encoder and its picture size.
    running: Option<(StageEncoder, FrameSize)>,
    memory:super::frame_memory::MemoryHold,
    admission:Option<Arc<super::capture_admission::SessionAdmission>>,
}

impl Pipeline {
    pub(crate) fn next_frame_id(&self) -> u64 { self.next_frame_id }
    pub(crate) fn set_visible_rects(&mut self, rects: Option<Vec<super::visibility::VisibleRect>>) {
        self.visible_rects=rects.filter(|rects|rects.len()<=super::visibility::MAX_VISIBLE_RECTS && rects.iter().all(|r|
            r.width>0 && r.height>0 && r.x.checked_add(r.width).is_some_and(|n|n<=self.source_size.width)
            && r.y.checked_add(r.height).is_some_and(|n|n<=self.source_size.height)));
    }
    fn visible_for_size(&self,size:FrameSize) -> Option<Vec<super::visibility::VisibleRect>> {
        if self.source_size.width==0 || self.source_size.height==0 {return None;}
        self.visible_rects.as_ref().map(|rects|rects.iter().map(|r| {
            let (sw,sh)=(u64::from(self.source_size.width),u64::from(self.source_size.height));
            let x=u64::from(r.x)*u64::from(size.width)/sw;let y=u64::from(r.y)*u64::from(size.height)/sh;
            let right=(u64::from(r.x+r.width)*u64::from(size.width)).div_ceil(sw);
            let bottom=(u64::from(r.y+r.height)*u64::from(size.height)).div_ceil(sh);
            super::visibility::VisibleRect {x:x as u32,y:y as u32,width:(right-x) as u32,height:(bottom-y) as u32}
        }).collect())
    }
    pub(crate) fn set_tile_limit(&mut self, size: FrameSize) { self.tile_limit=Some(size); }

    /// P6/route/source/decoder changes restart both the codec and composite reference.
    pub(crate) fn invalidate_reference(&mut self) {
        if self.gop.is_none() && self.running.is_none() && self.pacer.frames_in_flight()==0 {
            self.sender.reset(); self.drop_encoder();
        } else { self.reset_stream(); }
    }

    pub(crate) fn totals(&self) -> Totals {
        self.totals
    }

    /// What `STATE` reports of the running picture stream.
    pub(crate) fn stream_info(&self) -> Option<StreamInfo> {
        let (encoder, size) = self.running.as_ref()?;
        Some(StreamInfo {
            codec: self.choice.codec,
            profile: self.choice.profile,
            size: *size,
            encoder: encoder.name.clone(),
            hardware: encoder.hardware,
        })
    }

    /// The source within the bounds of the mode and of the codec.
    fn output_limits(&self)->io::Result<super::frame_budget::FrameLimits> {
        match self.frame_contract {
            Some(contract)=>Ok(contract.limits),
            None=>self.admission.as_ref().map(|admission|
                super::frame_budget::legacy_limits(admission.working_bytes(),None)).unwrap_or(Ok(super::frame_budget::FrameLimits::LEGACY)),
        }
    }
    fn fitted_size(&self) -> FrameSize {
        let Some(stage) = &self.stage else {
            return self.quality.size;
        };
        let max = self.choice.max_size;
        let limit = FrameSize::new(
            self.bounds.width.min(max.width),
            self.bounds.height.min(max.height),
        );
        let size = stage.source.size().fit_within(limit);
        let Ok(limits)=self.output_limits() else {return FrameSize::new(0,0);};
        let mut limits=limits.intersect(stage.source.frame_limits_for_contract(self.frame_contract));
        if self.frame_contract.is_some() {
            limits.max_pixels=limits.max_pixels.min(limits.working_bytes/super::frame_budget::VERSIONED_WORKING_IMAGES/4);
        }
        limits.max_pixels = limits.max_pixels.min(self.choice.max_pixels);
        let fitted=limits.fit(size);
        if self.choice.codec==CodecKind::H264 {super::frame_size_caps::fit_macroblocks(fitted,limits.max_pixels/256)} else {fitted}
    }

    /// The size of the next picture group: tiles keep the fitted source,
    /// H.264 keeps those pixels with even sides; cadence absorbs low rates.
    fn target_size(&self) -> FrameSize {
        let fitted = self.fitted_size();
        match self.choice.codec {
            CodecKind::Tiles => fitted,
            CodecKind::H264 => {
                self.quality.size.fit_within(fitted).even()
            }
        }
    }

    /// Ends the current picture group and its encoder; the next frame is a
    /// key frame of a new encoder.
    fn drop_encoder(&mut self) {
        self.work.invalidate_reference();
        self.presentation.invalidate(self.next_frame_id);
        // The old GOP's receipt cannot hold a restarted preview hostage.
        self.preview_in_flight = None;
        self.preview_sent_us = 0;
        if let Some(stage) = &mut self.stage {
            stage.encoder = None;
        }
        self.running = None;
        self.encoder_epoch += 1;
        if let Some(gop) = self.gop.take() {
            gop.finish();
        }
        self.pacer.request_keyframe();
    }

    /// Starts a new picture group when the target size changed.
    fn resize(&mut self) {
        let size = self.target_size();
        let same = self
            .running
            .as_ref()
            .is_some_and(|(_, running)| *running == size);
        if !same {
            self.drop_encoder();
        }
    }

    /// Gives the current picture stream up; the next frame is a key frame.
    fn reset_stream(&mut self) {
        if let Some(gop) = self.gop.take() {
            gop.reset();
        }
        self.pacer.on_stream_reset();
        self.sender.reset();
        self.drop_encoder();
    }


}

#[path="host_pipeline_init.rs"]
mod initialization;
#[path="host_pipeline_produce.rs"]
mod producing;
