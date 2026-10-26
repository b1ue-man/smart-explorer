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
use super::host_backend::{EncodedFrame, ScreenHostBackend, ScreenSource};
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
use capture::{produce, Captured, Stage};
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
}

impl Pipeline {
    pub(crate) fn new(
        backend: Arc<dyn ScreenHostBackend>,
        source: Box<dyn ScreenSource>,
        choice: CodecChoice,
        connection: &Connection,
        max_fps: u32,
        display: Option<FrameSize>,
        inject_resets: Arc<AtomicU32>,
    ) -> Self {
        let source_size=source.size();
        let source_generation=source.reference_generation();
        let quality_policy = QualityPolicy::new(choice, max_fps);
        let max_fps = quality_policy.max_fps();
        let (path, _) = path_of(connection);
        let display = display.unwrap_or(FrameSize::MAX);
        let mut pipeline = Self {
            work:Arc::new(work_control::WorkControl::default()),
            backend,
            frame_contract: None,
            negotiated_contract: None,
            sender: super::sender_budget::SenderBudget::for_peer(connection.remote_id().to_string()),
            route_epoch: None,
            source_size,source_generation,presentation:ReferencePresentation::new(),
            capture_active: true, capture_applied:Some(true), capture_requested:true,capture_ready:None,capture_transition:None,visible_rects: None,
            stage: Some(Stage {
                source,
                encoder: None,
            }),
            choice,
            quality_policy,
            fallback_notice: None,
            tile_limit: (choice.codec==CodecKind::Tiles).then_some(choice.max_size),
            kind: StreamKind::Video,
            gop: None,
            next_gop_id: 1,
            next_frame_id: 1,
            pacer: Pacer::default(),
            rate: RateController::new(path, now_us()),
            clock: FrameClock::new(max_fps),codec_fps:0,
            quality: Quality {
                fps: max_fps,
                size: FrameSize::MAX,
            },
            max_fps,
            display,
            bounds: display,
            preview_interval_us: PREVIEW_INTERVAL_US,
            preview_in_flight: None,
            preview_sent_us: 0,
            totals: Totals::default(),
            second_bytes: 0,
            second_frames: 0,
            inject_resets,
            encoder_epoch: 0,
            running: None,
        };
        pipeline.quality.size = pipeline.fitted_size();
        pipeline.resize();
        pipeline
    }

    pub(crate) fn set_frame_contract(&mut self, contract: Option<super::frame_budget::FrameContract>) -> io::Result<()> {
        if let Some(contract) = contract { contract.validate()?; self.bounds = contract.limits.max_size; self.display = self.bounds; }
        self.negotiated_contract = contract;
        self.frame_contract = contract.map(|mut c| {c.hybrid_regions &= self.stage.as_ref().is_some_and(|stage|stage.source.supports_frame_regions());c}); self.quality.size = self.fitted_size(); self.invalidate_reference(); Ok(())
    }

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
        let mut limits = self.frame_contract.map_or(super::frame_budget::FrameLimits::LEGACY, |c| c.limits)
            .intersect(stage.source.frame_limits_for_contract(self.frame_contract));
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

    /// One capture tick.
    pub(crate) async fn tick(
        &mut self,
        connection: &Connection,
    ) -> Result<Produced, PipelineError> {
        self.tick_epoch(connection,self.work.epoch()).await
    }
    pub(super) async fn tick_epoch(&mut self,connection:&Connection,work_epoch:u64)->Result<Produced,PipelineError> {
        if !self.work.current(work_epoch) {self.reset_stream();return Ok(Produced::Reset);}
        if !self.capture_active || self.capture_transition.is_some() { return Ok(Produced::Idle); }
        if let Some(route) = crate::share::route_monitor::snapshot(connection) {
            let epoch = (route.generation,route.pause_epoch,route.class);
            if self.route_epoch != Some(epoch) { self.invalidate_reference(); self.route_epoch = Some(epoch); }
            if !route.application_allowed { return Ok(Produced::Idle); }
        }
        self.sync_source_reference();
        let fitted=self.fitted_size();
        let size=self.target_size();
        if self.choice.codec==CodecKind::H264 && (fitted.width<2 || fitted.height<2) {
            self.fall_back_to_tiles();return Ok(Produced::Reset);
        }
        let rate=if self.choice.codec==CodecKind::H264 {self.choice.max_macroblocks_per_second} else {None};
        let Some(fps)=self.clock.set_codec_fps(self.quality.fps,size,rate) else {
            self.fall_back_to_tiles();return Ok(Produced::Reset);
        };
        let changed=self.codec_fps!=fps;self.codec_fps=fps;
        if changed && self.running.is_some() {self.drop_encoder();return Ok(Produced::Reset);}
        let now = now_us();
        if !self.clock.take(now) {
            return Ok(Produced::Idle);
        }
        if self.kind == StreamKind::Preview
            && (self.preview_in_flight.is_some()
                || now.saturating_sub(self.preview_sent_us) < self.preview_interval_us)
        {
            return Ok(Produced::Idle);
        }
        let (path, rtt_us) = path_of(connection);
        self.rate.set_path(path);
        self.rate.set_timing(rtt_us,self.clock.interval_us());
        let target = self.rate.target_bps();
        let keyframe = loop {
            match self.pacer.decide(now, target, path, rtt_us) {
                PaceDecision::Encode { keyframe } => break keyframe,
                PaceDecision::DropBeforeEncode => return Ok(Produced::Dropped),
                PaceDecision::ResetStream => self.reset_stream(),
            }
        };
        if self.gop.is_none() && !keyframe {
            self.pacer.request_keyframe();
            return Ok(Produced::Idle);
        }
        let Some(permit) = self.sender.admit(target,path,rtt_us,now,keyframe) else { self.pacer.on_budget_drop(); return Ok(Produced::Dropped); };
        let size = self.target_size();
        let mut stage = self
            .stage
            .take()
            .ok_or_else(|| PipelineError::capture(io::Error::other("no screen source")))?;
        let request = StageRequest {
            frame_id: self.next_frame_id,
            frame_contract: self.frame_contract,
            visible_rects: self.visible_for_size(size),
            keyframe,
            target_bps: target,
            size,
            choice: self.choice,
            encoder_epoch: self.encoder_epoch,
            fps,
        };
        let backend = self.backend.clone();
        let encoding_started = std::time::Instant::now();
        let joined = tokio::task::spawn_blocking(move || {
            let produced = produce(&mut stage, backend.as_ref(), &request);
            (stage, produced, request)
        })
        .await;
        let (stage, produced, request) =
            joined.map_err(|error| PipelineError::capture(io::Error::other(error)))?;
        self.stage = Some(stage);
        if !self.work.current(work_epoch) {self.reset_stream();return Ok(Produced::Reset);}
        if self.sync_source_reference() {
            return match produced {Ok(Captured::Paused(state))=>Ok(Produced::Paused(state)),Err(error)=>Err(error),_=>Ok(Produced::Reset)};
        }
        match produced {
            Ok(Captured::Unchanged) => Ok(Produced::Idle),
            Ok(Captured::Paused(state)) => Ok(Produced::Paused(state)),
            Ok(Captured::Encoded {
                frame,
                capture_ts_us,
                encoder,
            }) => {
                let encoded_bytes = frame.records.iter().map(|r|r.len() as u64).sum();
                self.rate.on_encode(encoded_bytes, encoding_started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64);
                self.running = Some((encoder, size));
                let frame_id = request.frame_id;
                self.send(connection, frame, frame_id, capture_ts_us, size, permit,work_epoch)
                    .await
            }
            Err(failure)
                if failure.code == ScreenErrorCode::EncoderFailed
                    && self.choice.codec != CodecKind::Tiles =>
            {
                self.quality_policy.encoder_failed = true;
                self.fallback_notice = Some(format!("encoder_fallback_to_tiles: {}", failure.error));
                self.fall_back_to_tiles();
                Ok(Produced::Reset)
            }
            Err(failure) => Err(failure),
        }
    }

}
