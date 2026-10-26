//! Capture ticks, metadata readiness and one ordered producer task.
use super::*;

impl Pipeline {
    /// One capture tick.
    pub(crate) async fn tick(
        &mut self,
        connection: &Connection,
    ) -> Result<Produced, PipelineError> {
        self.tick_epoch(connection, self.work.epoch()).await
    }
    pub(super) async fn tick_epoch(
        &mut self,
        connection: &Connection,
        work_epoch: u64,
    ) -> Result<Produced, PipelineError> {
        if !self.work.current(work_epoch) {
            self.reset_stream();
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
        if self.choice.codec == CodecKind::H264 && (fitted.width < 2 || fitted.height < 2) {
            self.fall_back_to_tiles();
            return Ok(Produced::Reset);
        }
        let rate = if self.choice.codec == CodecKind::H264 {
            self.choice.max_macroblocks_per_second
        } else {
            None
        };
        let Some(fps) = self.clock.set_codec_fps(self.quality.fps, size, rate) else {
            self.fall_back_to_tiles();
            return Ok(Produced::Reset);
        };
        let changed = self.codec_fps != fps;
        self.codec_fps = fps;
        if changed && self.running.is_some() {
            self.drop_encoder();
            return Ok(Produced::Reset);
        }
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
        self.rate.set_timing(rtt_us, self.clock.interval_us());
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
        let Some(permit) = self.sender.admit(target, path, rtt_us, now, keyframe) else {
            self.pacer.on_budget_drop();
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
            StageOutput {
                result: (produced, request),
                stage,
            }
        })
        .await;
        let StageOutput {
            stage,
            result: (produced, request),
        } = joined.map_err(|error| PipelineError::capture(io::Error::other(error)))?;
        self.stage = Some(stage);
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
                self.rate.on_encode(
                    encoded_bytes,
                    encoding_started
                        .elapsed()
                        .as_micros()
                        .min(u128::from(u64::MAX)) as u64,
                );
                self.running = Some((encoder, size));
                let frame_id = request.frame_id;
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
            Err(failure)
                if failure.code == ScreenErrorCode::EncoderFailed
                    && self.choice.codec != CodecKind::Tiles =>
            {
                self.quality_policy.encoder_failed = true;
                self.fallback_notice =
                    Some(format!("encoder_fallback_to_tiles: {}", failure.error));
                self.fall_back_to_tiles();
                Ok(Produced::Reset)
            }
            Err(failure) => Err(failure),
        }
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
