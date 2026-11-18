//! Capture ticks, metadata readiness and one ordered producer task.
//!
//! A stage that paces itself (the host process, plan „Bildkette“ B-27) is
//! not gated by the daemon's frame clock: it captures at the cadence it gets
//! with every order, and the session asks for its next picture as soon as
//! the previous one is written (`chain_ready`, BK3b). Pacer and sender
//! budget still decide before every credit; a refused credit waits for an
//! acknowledgement or the next tick.
use super::*;
use crate::screen::core::host_stage::{StreamHint, StreamReport};
use crate::screen::core::pacing::codec_cap;
use crate::screen::core::rate_control::model::start_table_bps;
use crate::screen::core::rate_control::{codec_ceiling_bps, PathKind};

/// Dropped pictures one report adds to the statistics at most.
const MAX_REPORTED_DROPS: u64 = 1 << 16;

impl Pipeline {
    /// One capture tick.
    pub async fn tick(&mut self, connection: &Connection) -> Result<Produced, PipelineError> {
        self.tick_epoch(connection, self.work.epoch()).await
    }
    pub(super) async fn tick_epoch(
        &mut self,
        connection: &Connection,
        work_epoch: u64,
    ) -> Result<Produced, PipelineError> {
        // State changes may go on at once; only an answer of the stage or a
        // change of state makes the next tick ready (BK3b).
        self.chain_ready = false;
        if !self.work.current(work_epoch) {
            self.reset_stream();
            self.chain_ready = true;
            return Ok(Produced::Reset);
        }
        if !self.capture_active || self.capture_transition.is_some() {
            return Ok(Produced::Idle);
        }
        if let Some(route) = crate::share::route_monitor::snapshot(connection) {
            let epoch = (route.generation, route.pause_epoch, route.class);
            if self.route_epoch != Some(epoch) {
                self.invalidate_reference();
                self.route_epoch = Some(epoch);
            }
            if !route.application_allowed {
                return Ok(Produced::Idle);
            }
        }
        self.sync_source_reference();
        if self.source_size.width == 0 || self.source_size.height == 0 {
            if !self.clock.take(now_us()) {
                return Ok(Produced::Idle);
            }
            return self.poll_prepared_source(work_epoch).await;
        }
        let fitted = self.fitted_size();
        let size = self.target_size();
        // Every video codec (H.264, HEVC, AV1; BK6 hand-over, merge W1)
        // needs at least 2 x 2 pixels and is bounded by its pairing's rate.
        let video = self.choice.codec.is_video();
        if video && (fitted.width < 2 || fitted.height < 2) {
            self.fall_back_to_tiles();
            self.chain_ready = true;
            return Ok(Produced::Reset);
        }
        let rate = if video {
            self.choice.max_macroblocks_per_second
        } else {
            None
        };
        // The encoder opens with the highest rate the viewer and the codec
        // pairing allow and then gets fewer pictures when the cadence sinks:
        // a new cadence keeps it (plan 2026-11-14 „Bildkette“ B-03). Only a
        // higher allowed rate than the one it was opened with restarts it.
        let Some(open_fps) = codec_cap(self.max_fps, size, rate) else {
            // Only a fallback that changes the codec lets the next tick
            // follow at once; the tiles without a size wait for the tick.
            self.chain_ready = video;
            self.fall_back_to_tiles();
            return Ok(Produced::Reset);
        };
        if self.running.is_some() && open_fps > self.codec_fps {
            self.drop_encoder();
            self.chain_ready = true;
            return Ok(Produced::Reset);
        }
        if self.running.is_none() {
            self.codec_fps = open_fps;
        }
        let fps = self.codec_fps;
        self.clock.set_fps(self.quality.fps.min(open_fps));
        self.rate
            .set_codec_ceiling(codec_ceiling_bps(&self.choice, size, open_fps));
        // Plan 2026-11-17: the stream's start rate (`rate_control::model`).
        let chroma_444 = self.choice.codec.chroma_444(self.choice.profile);
        self.rate.set_stream_start(start_table_bps(
            size,
            open_fps,
            self.choice.codec,
            chroma_444,
        ));
        let now = now_us();
        self.self_paced = self
            .stage
            .as_mut()
            .and_then(|stage| stage.source.host_stage())
            .is_some_and(|host| host.self_paced());
        // A self-paced stage keeps the cadence itself; the daemon's clock
        // would only round its pictures to the session loop's ticks.
        if !self.self_paced && !self.clock.take(now) {
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
        if !self.rate.peer_bound() {
            self.rate.bind_peer(&connection.remote_id().to_string());
        }
        self.rate.set_timing(rtt_us, self.clock.interval_us());
        let mut target = self.rate.target_bps();
        // B-35: the stale time follows the smoothed round trip of the path
        // (`path_of` reads noq's smoothed estimate) and the cadence.
        self.pacer.set_timing(rtt_us, self.clock.interval_us());
        // The unacknowledged bytes drain at the measured rate (plan 2026-11-17).
        self.pacer.set_drain_rate(self.rate.sustained_bps());
        // P9: frames wait for at most what the line delivers over the
        // round-trip floor plus two frames (`rate_control::queue`).
        let base_rtt_us = self.rate.base_rtt_us(now).unwrap_or(rtt_us);
        self.pacer
            .set_queue(self.rate.queue_rate_bps(now), base_rtt_us);
        // A frame the line cannot take is dropped before encoding: for the
        // tile codec, whose size no target steers, this is how its frame rate
        // sinks to what the line carries; for every codec it is back pressure.
        let keyframe = loop {
            match self.pacer.decide(now, target, path, rtt_us) {
                PaceDecision::Encode { keyframe } => break keyframe,
                PaceDecision::DropBeforeEncode => {
                    self.rate.on_back_pressure(now);
                    return Ok(Produced::Dropped);
                }
                PaceDecision::ResetStream => {
                    // The stale stream lowered the target: the frame that
                    // follows is sized for the lowered one.
                    self.reset_stale_stream();
                    target = self.rate.target_bps();
                }
            }
        };
        // B-36: a new stream without a key frame only as a continuation.
        let recover = self.continue_from.filter(|_| !keyframe);
        if self.gop.is_none() && !keyframe && recover.is_none() {
            self.pacer.request_keyframe();
            self.chain_ready = true;
            return Ok(Produced::Idle);
        }
        let Some(permit) = self.sender.admit(target, path, rtt_us, now, keyframe) else {
            self.pacer.on_budget_drop();
            self.rate.on_back_pressure(now);
            return Ok(Produced::Dropped);
        };
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
            // The encoder gets the target times its correction (plan 2026-11-17).
            target_bps: self.rate.encoder_bps(),
            size,
            choice: self.choice,
            encoder_epoch: self.encoder_epoch,
            fps,
        };
        let backend = self.backend.clone();
        // A host process paces itself (plan „Bildkette“ B-27): it captures
        // at the session's cadence and may encode the next picture while
        // this one is sent, when the queue has room for both.
        let hint = StreamHint {
            cadence_fps: (1_000_000 / self.clock.interval_us().max(1)).min(u64::from(u32::MAX))
                as u32,
            ahead: self.ahead_allowed(target, path, rtt_us),
        };
        let encoding_started = std::time::Instant::now();
        let joined = tokio::task::spawn_blocking(move || {
            if let Some(host) = stage.source.host_stage() {
                host.set_stream_hint(hint);
            }
            let produced = produce(&mut stage, backend.as_ref(), &request, recover);
            let report = stage
                .source
                .host_stage()
                .and_then(|host| host.take_stream_report());
            StageOutput {
                result: (produced, request, report),
                stage,
            }
        })
        .await;
        let StageOutput {
            stage,
            result: (produced, request, report),
        } = joined.map_err(|error| PipelineError::capture(io::Error::other(error)))?;
        self.stage = Some(stage);
        // The stage answered its credit: the next one may follow at once.
        self.chain_ready = true;
        // Pictures the host process captured and replaced before encoding
        // count with the frames the queue guard dropped (`HostStats`).
        for _ in 0..report.map_or(0, |report| {
            report.dropped_before_encode.min(MAX_REPORTED_DROPS)
        }) {
            self.pacer.on_budget_drop();
        }
        if !self.work.current(work_epoch) {
            self.reset_stream();
            return Ok(Produced::Reset);
        }
        if self.sync_source_reference() {
            return match produced {
                Ok(Captured::Paused(state)) => Ok(Produced::Paused(state)),
                Err(error) => Err(error),
                _ => Ok(Produced::Reset),
            };
        }
        match produced {
            Ok(Captured::Unchanged) => Ok(Produced::Idle),
            Ok(Captured::Paused(state)) => Ok(Produced::Paused(state)),
            Ok(Captured::Encoded {
                frame,
                capture_ts_us,
                encoder,
            }) => {
                let encoded_bytes = frame.records.iter().map(|r| r.len() as u64).sum();
                let waited = encoding_started
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64;
                self.rate
                    .on_encode_at(encoded_bytes, production_us(report, waited), now_us());
                // Whether the video encoder keeps the rate it was given (plan
                // 2026-11-17); lossless regions of a hybrid stream are no part
                // of its rate.
                let hybrid = request.frame_contract.is_some_and(|c| c.hybrid_regions);
                if request.choice.codec.is_video() && !hybrid {
                    self.rate.on_encoded(
                        encoded_bytes,
                        frame.keyframe,
                        target,
                        request.target_bps,
                        request.fps,
                        &encoder.name,
                    );
                }
                // Mid-GOP encoder fallback (W4, B3): start a fresh picture group
                // so the new encoder's parameter sets never mix with the old ones.
                if self
                    .running
                    .as_ref()
                    .is_some_and(|(previous, _)| previous.name != encoder.name)
                {
                    if let Some(gop) = self.gop.take() {
                        gop.finish();
                    }
                    self.confirmed.restart();
                    self.continue_from = None;
                }
                self.running = Some((encoder, size));
                // The host process drew the picture with the id it expected
                // for it; the ids stay increasing either way.
                let frame_id = report
                    .and_then(|report| report.frame_id)
                    .map_or(request.frame_id, |id| id.max(request.frame_id));
                self.next_frame_id = frame_id;
                self.send(
                    connection,
                    frame,
                    frame_id,
                    capture_ts_us,
                    size,
                    permit,
                    work_epoch,
                )
                .await
            }
            // Merge W2: the next negotiated codec first (HEVC → H.264), the
            // lossless tiles only when no other one is left.
            Err(failure)
                if failure.code == ScreenErrorCode::EncoderFailed
                    && self.choice.codec != CodecKind::Tiles =>
            {
                if let Some(next) = self.fall_back_to_next_codec() {
                    self.fallback_notice = Some(format!(
                        "encoder_fallback_to_{}: {}",
                        next.codec.key(),
                        failure.error
                    ));
                    return Ok(Produced::Reset);
                }
                self.quality_policy.note_failed_codec(self.choice.codec);
                self.quality_policy.encoder_failed = true;
                self.fallback_notice =
                    Some(format!("encoder_fallback_to_tiles: {}", failure.error));
                self.fall_back_to_tiles();
                Ok(Produced::Reset)
            }
            Err(failure) => Err(failure),
        }
    }

    /// One picture may be encoded ahead of its frame when the unacknowledged
    /// bytes plus this frame and the next one (each the session's mean frame
    /// size) stay under the queue guard of the pacer (`Pacer::guard`, P9);
    /// a preview never does.
    fn ahead_allowed(&self, target: u64, path: PathKind, rtt_us: u64) -> bool {
        if self.kind == StreamKind::Preview || self.totals.frames == 0 {
            return false;
        }
        let mean = self.totals.bytes / self.totals.frames;
        self.pacer
            .unacked_bytes()
            .saturating_add(mean.saturating_mul(2))
            <= self.pacer.guard(target, path, rtt_us)
    }

    async fn poll_prepared_source(&mut self, work_epoch: u64) -> Result<Produced, PipelineError> {
        let mut stage = self
            .stage
            .take()
            .ok_or_else(|| PipelineError::capture(io::Error::other("no screen preparation")))?;
        let request = StageRequest {
            frame_id: self.next_frame_id,
            frame_contract: self.frame_contract,
            visible_rects: None,
            keyframe: true,
            target_bps: self.rate.target_bps(),
            size: FrameSize::new(0, 0),
            choice: self.choice,
            encoder_epoch: self.encoder_epoch,
            fps: self.max_fps,
        };
        let job = tokio::task::spawn_blocking(move || {
            let result = stage.source.prepare_capture(&request);
            StageOutput { result, stage }
        })
        .await
        .map_err(|error| PipelineError::capture(io::Error::other(error)))?;
        self.stage = Some(job.stage);
        if !self.work.current(work_epoch) {
            self.reset_stream();
            return Ok(Produced::Reset);
        }
        self.sync_source_reference();
        match job.result {
            Ok(Some(state)) => Ok(Produced::Paused(state)),
            Ok(None) if self.source_size.width != 0 && self.source_size.height != 0 => {
                Ok(Produced::Reset)
            }
            Ok(None) => Ok(Produced::Paused(SessionState::Starting)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                Ok(Produced::Paused(SessionState::Starting))
            }
            Err(error) => Err(PipelineError::capture(error)),
        }
    }
}

/// The production time of one picture for the frame rate (B-02): with a
/// self-paced host process the slower of its two stages, which run side by
/// side; otherwise the time `produce` took.
fn production_us(report: Option<StreamReport>, waited_us: u64) -> u64 {
    match report {
        Some(report) if report.capture_us != 0 || report.encode_us != 0 => {
            report.capture_us.max(report.encode_us)
        }
        _ => waited_us,
    }
}
