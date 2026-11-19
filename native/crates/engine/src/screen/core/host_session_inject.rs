//! The end of a batch's injection (review 2026-11-19, Codex 7/10): the
//! session loop takes it from `settle_input` on every viewer frame and
//! capture tick, applies it and starts the batches that waited, in order.
use super::*;

impl Session<'_> {
    /// Takes the result of a finished injection and starts the batches that
    /// waited, in their order (on every viewer frame and capture tick).
    pub(in super::super) async fn settle_input(&mut self) -> io::Result<()> {
        loop {
            if let Some(job) = self.health.injecting.as_mut() {
                use std::future::Future;
                let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
                let applied = match std::pin::Pin::new(&mut job.handle).poll(&mut cx) {
                    std::task::Poll::Pending => return Ok(()),
                    std::task::Poll::Ready(result) => {
                        result.unwrap_or(Err(ControlBlock::ControlUnavailable))
                    }
                };
                let Some(job) = self.health.injecting.take() else {
                    return Ok(());
                };
                self.applied(job, applied).await?;
                // Releases that waited behind the injection go now.
                self.flush_releases();
            }
            let Some(next) = self.health.backlog.pop_front() else {
                return Ok(());
            };
            self.start_batch(&next).await?;
        }
    }

    /// What a finished injection means for the session and its viewer.
    pub(super) async fn applied(
        &mut self,
        job: super::super::input_release::Injection,
        applied: Result<(), ControlBlock>,
    ) -> io::Result<()> {
        let super::super::input_release::Injection {
            events,
            reference,
            presented,
            started,
            ..
        } = job;
        let events = &events[..];
        let sink = self.pipeline.input_sink();
        // Station `input_inject` (BK0): the injection of this batch.
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        crate::screen::core::chain_meter::lock(self.pipeline.chain_meter()).record_us(
            crate::screen::core::chain_meter::Station::InputInject,
            micros,
        );
        match applied {
            Ok(()) => {
                for event in events {
                    self.pressed.observe(event);
                }
                self.health.unconfirmed = None;
                // D6/E2: a block or hold the viewer heard ends with the
                // first batch that reached the host after it; a block the
                // platform reported late stays shown its minimum time (Z2).
                let held = std::mem::take(&mut self.health.held_reported);
                let late = self
                    .health
                    .block_shown_until
                    .is_some_and(|until| Instant::now() < until);
                let cleared = !late && self.health.block_reported.take().is_some();
                if cleared || held {
                    let state = ControlState {
                        holder: self.handle.controller(),
                        blocked: self.health.block_reported,
                        held: None,
                    };
                    self.frame(HostFrame::ControlState(state)).await?;
                }
                Ok(())
            }
            Err(blocked) => {
                // S3: a refusal for a reference the source just left holds
                // the batch back and keeps what the user holds; the new
                // reference lets go of it when it moves the floor.
                if presented && self.stale_refusal(blocked, reference) {
                    // Held back, not blocked (review O2a); its releases of
                    // held keys go in (E2: the viewer sends each once).
                    self.release_held_ups(events);
                    return self.report_held(InputHeld::WaitingForPicture).await;
                }
                self.release_pressed();
                self.report_block(blocked).await?;
                if !presented
                    && sink
                        .as_ref()
                        .is_some_and(|sink| sink.legacy_input_invalid())
                {
                    self.error(ScreenErrorCode::ControlUnavailable,
                        "Der Bildausschnitt hat sich geändert. Aufnahme läuft weiter; zum Steuern ist eine neue Verbindung erforderlich.".into()).await
                } else {
                    Ok(())
                }
            }
        }
    }
}
