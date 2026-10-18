//! The picture side of one host session (decisions C2, C3, C6): the frame
//! clock takes captured frames at the wanted rate; the pacer decides before
//! capture and encoding whether a frame fits the queue guard (`Pacer::decide`
//! with the path and round trip of the connection), resets a stale picture
//! stream and asks for key frames; capture and encoding run on the blocking
//! pool; the records go to the picture stream of the current picture group,
//! stamped after the last one (`SentFrame.sent_us`). `FRAME_ACK` feeds pacer
//! and rate control, the cumulative loss counters arrive once a second. A
//! picture stream that does not take a frame within the stale time, or fails,
//! is reset and the next frame is a key frame on a new stream
//! (`Pacer::on_stream_reset`). Previews (C6) travel one at a time: the next
//! one only after `PREVIEW_ACK` and the chosen interval.
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;

use super::gop_stream::{loss_counters, now_us, path_of, GopWriter};
use super::host_backend::{EncodedFrame, ScreenHostBackend, ScreenSource};
use super::negotiate::{CodecChoice, CodecKind};
use super::pacing::{FrameClock, PaceDecision, Pacer, SentFrame, STALE_STREAM_US};
use super::pixel_convert::FrameSize;
use super::rate_control::{adjust_quality, AckSample, Quality, RateController};
use super::session_state::SessionState;
use super::wire_binary::{GopHeader, StreamKind};
use super::wire_control::{FrameAck, HostStats, StreamInfo};
use super::wire_hello::ScreenErrorCode;

#[path = "host_capture.rs"]
mod capture;
use capture::{capture_and_encode, Captured, Stage};

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

/// Counters of one session for the statistics and the access log.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Totals {
    pub bytes: u64,
    pub frames: u64,
    pub keyframes: u64,
}

pub(crate) struct Pipeline {
    backend: Arc<dyn ScreenHostBackend>,
    stage: Option<Stage>,
    choice: CodecChoice,
    kind: StreamKind,
    gop: Option<GopWriter>,
    next_gop_id: u64,
    next_frame_id: u64,
    pacer: Pacer,
    rate: RateController,
    clock: FrameClock,
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
        let (path, _) = path_of(connection);
        let display = display.unwrap_or(FrameSize::MAX);
        let mut pipeline = Self {
            backend,
            stage: Some(Stage {
                source,
                encoder: None,
            }),
            choice,
            kind: StreamKind::Video,
            gop: None,
            next_gop_id: 1,
            next_frame_id: 1,
            pacer: Pacer::default(),
            rate: RateController::new(path, now_us()),
            clock: FrameClock::new(max_fps),
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
        };
        pipeline.quality.size = pipeline.fitted_size();
        pipeline.resize();
        pipeline
    }

    pub(crate) fn totals(&self) -> Totals {
        self.totals
    }

    /// What `STATE` reports of the running picture stream.
    pub(crate) fn stream_info(&self) -> Option<StreamInfo> {
        let encoder = self.stage.as_ref()?.encoder.as_ref()?;
        Some(StreamInfo {
            codec: self.choice.codec,
            profile: self.choice.profile,
            size: encoder.size(),
            encoder: encoder.name(),
            hardware: self.choice.encoder_hardware,
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
        stage.source.size().fit_within(limit)
    }

    /// The size of the next picture group: tiles keep the fitted source,
    /// H.264 takes the step of the quality ladder with even sides.
    fn target_size(&self) -> FrameSize {
        let fitted = self.fitted_size();
        match self.choice.codec {
            CodecKind::Tiles => fitted,
            CodecKind::H264 => self.quality.size.fit_within(fitted).even(),
        }
    }

    /// Starts a new picture group when the target size changed.
    fn resize(&mut self) {
        let size = self.target_size();
        let Some(stage) = &mut self.stage else {
            return;
        };
        if stage
            .encoder
            .as_ref()
            .is_some_and(|encoder| encoder.size() == size)
        {
            return;
        }
        stage.encoder = None;
        if let Some(gop) = self.gop.take() {
            gop.finish();
        }
        self.pacer.request_keyframe();
    }

    /// Ends the current picture group; the next frame is a key frame.
    fn restart(&mut self) {
        if let Some(stage) = &mut self.stage {
            stage.encoder = None;
        }
        if let Some(gop) = self.gop.take() {
            gop.finish();
        }
        self.pacer.request_keyframe();
    }

    /// `SET_MODE`: the stream kind, the size the viewer wants and the rate.
    pub(crate) fn set_mode(&mut self, kind: StreamKind, size: Option<FrameSize>, fps: Option<u16>) {
        if kind != self.kind {
            self.kind = kind;
            self.preview_in_flight = None;
            self.restart();
        }
        self.bounds = match (kind, size) {
            (_, Some(size)) => size,
            (StreamKind::Preview, None) => FrameSize::new(PREVIEW_EDGE, PREVIEW_EDGE),
            _ => self.display,
        };
        if let Some(fps) = fps {
            let fps = u32::from(fps).max(1);
            if kind == StreamKind::Preview {
                self.preview_interval_us = 1_000_000 / u64::from(fps);
            } else {
                self.max_fps = fps;
                self.quality.fps = self.quality.fps.min(fps);
                self.clock.set_fps(self.quality.fps);
            }
        }
        self.quality.size = self.fitted_size();
        self.resize();
    }

    /// Replaces the source (`SET_MODE` with another source).
    pub(crate) fn set_source(&mut self, source: Box<dyn ScreenSource>) {
        self.stage = Some(Stage {
            source,
            encoder: None,
        });
        self.quality.size = self.fitted_size();
        self.restart();
    }

    pub(crate) fn request_keyframe(&mut self) {
        self.pacer.request_keyframe();
    }

    /// `FRAME_ACK` or `PREVIEW_ACK`.
    pub(crate) fn on_ack(&mut self, ack: FrameAck) {
        if self.preview_in_flight == Some(ack.frame_id) {
            self.preview_in_flight = None;
        }
        if let Some(sent) = self.pacer.on_ack(ack.gop_id, ack.frame_id) {
            let sample = AckSample {
                send_us: sent.sent_us,
                recv_us: ack.received_us,
                bytes: sent.bytes,
                keyframe: sent.keyframe,
            };
            self.rate.on_frame_acked(sample, now_us());
        }
    }

    /// Once a second: loss counters, quality ladder and the statistics.
    pub(crate) fn second(&mut self, connection: &Connection) -> HostStats {
        let (sent, lost) = loss_counters(connection);
        let _ = self.rate.on_loss_counters(sent, lost);
        let (path, _) = path_of(connection);
        self.rate.set_path(path);
        let fitted = self.fitted_size();
        let next = adjust_quality(self.quality, self.rate.target_bps(), fitted, self.max_fps);
        if next.fps != self.quality.fps {
            self.clock.set_fps(next.fps);
        }
        self.quality = next;
        self.resize();
        let stats = HostStats {
            target_kbps: self.rate.target_bps() / 1000,
            sent_kbps: self.second_bytes * 8 / 1000,
            fps: self.second_frames,
            path,
            keyframes: self.totals.keyframes,
            dropped_before_encode: self.pacer.dropped_before_encode(),
            stream_resets: self.pacer.resets(),
            unacked_bytes: self.pacer.unacked_bytes(),
        };
        self.second_bytes = 0;
        self.second_frames = 0;
        stats
    }

    /// Gives the current picture stream up; the next frame is a key frame.
    fn reset_stream(&mut self) {
        if let Some(gop) = self.gop.take() {
            gop.reset();
        }
        self.pacer.on_stream_reset();
    }

    /// One capture tick.
    pub(crate) async fn tick(
        &mut self,
        connection: &Connection,
    ) -> Result<Produced, PipelineError> {
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
        let size = self.target_size();
        let mut stage = self
            .stage
            .take()
            .ok_or_else(|| PipelineError::capture(io::Error::other("no screen source")))?;
        if stage.encoder.is_none() {
            match self.backend.open_encoder(&self.choice, size) {
                Ok(encoder) => stage.encoder = Some(encoder),
                Err(error) => {
                    self.stage = Some(stage);
                    return Err(PipelineError::encoder(error));
                }
            }
        }
        let frame_id = self.next_frame_id;
        let joined = tokio::task::spawn_blocking(move || {
            let captured = capture_and_encode(&mut stage, size, frame_id, keyframe, target);
            (stage, captured)
        })
        .await;
        let (stage, captured) =
            joined.map_err(|error| PipelineError::capture(io::Error::other(error)))?;
        self.stage = Some(stage);
        match captured? {
            Captured::Unchanged => Ok(Produced::Idle),
            Captured::Paused(state) => Ok(Produced::Paused(state)),
            Captured::Encoded(frame, capture_ts_us) => {
                self.send(connection, frame, frame_id, capture_ts_us, size)
                    .await
            }
        }
    }

    async fn send(
        &mut self,
        connection: &Connection,
        frame: EncodedFrame,
        frame_id: u64,
        capture_ts_us: u64,
        size: FrameSize,
    ) -> Result<Produced, PipelineError> {
        if frame.records.is_empty() {
            return Ok(Produced::Idle);
        }
        let stale = Duration::from_micros(STALE_STREAM_US);
        if self.gop.is_none() {
            let header = GopHeader {
                gop_id: self.next_gop_id,
                kind: self.kind,
                codec: self.choice.codec,
                profile: self.choice.profile,
                size,
            };
            self.next_gop_id += 1;
            match tokio::time::timeout(stale, GopWriter::open(connection, header)).await {
                Ok(Ok(gop)) => self.gop = Some(gop),
                _ if connection.close_reason().is_some() => return Ok(Produced::Closed),
                // No free stream slot or credit yet: the frame is lost, the
                // next one is a key frame on a new stream.
                _ => {
                    self.pacer.on_stream_reset();
                    return Ok(Produced::Reset);
                }
            }
        }
        let Some(gop) = self.gop.as_mut() else {
            return Ok(Produced::Idle);
        };
        let gop_id = gop.header().gop_id;
        self.next_frame_id += 1;
        let written = tokio::time::timeout(
            stale,
            gop.write_frame(frame_id, capture_ts_us, frame.keyframe, frame.records),
        )
        .await;
        let bytes = match written {
            Ok(Ok(bytes)) => bytes,
            _ if connection.close_reason().is_some() => return Ok(Produced::Closed),
            _ => {
                self.reset_stream();
                return Ok(Produced::Reset);
            }
        };
        let sent_us = now_us();
        self.pacer.on_frame_sent(SentFrame {
            gop_id,
            frame_id,
            bytes,
            sent_us,
            keyframe: frame.keyframe,
        });
        self.rate.on_frame_sent(bytes, sent_us);
        self.totals.bytes += bytes;
        self.totals.frames += 1;
        self.totals.keyframes += u64::from(frame.keyframe);
        self.second_bytes += bytes;
        self.second_frames += 1;
        if self.kind == StreamKind::Preview {
            self.preview_in_flight = Some(frame_id);
            self.preview_sent_us = sent_us;
        }
        let inject = self
            .inject_resets
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        if inject {
            self.reset_stream();
            return Ok(Produced::Reset);
        }
        Ok(Produced::Sent)
    }
}
