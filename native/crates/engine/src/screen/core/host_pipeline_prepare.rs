//! A source still preparing its capture (no size yet): the stage is asked
//! to prepare once per tick of the frame clock (`Pipeline::tick_epoch`).
use super::super::*;

impl Pipeline {
    pub(super) async fn poll_prepared_source(
        &mut self,
        work_epoch: u64,
    ) -> Result<Produced, PipelineError> {
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
