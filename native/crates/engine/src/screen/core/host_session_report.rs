//! What the viewer hears of input the host did not inject: a block
//! (`CONTROL_STATE.blocked`, D6), a hold (`held`, E2/S4), a refusal the
//! platform reported after the batch was queued, and the refusal that may
//! only name a reference the source just left (D2).
use super::*;

/// How long a block the platform reported after the batch stays shown (Z2):
/// at least one look at the toolbar [assumption: 3 s, not measured].
const LATE_BLOCK_SHOWN: Duration = Duration::from_secs(3);

impl Session<'_> {
    /// An `ERROR` for the viewer; the session goes on.
    pub(in super::super) async fn error(
        &mut self,
        code: ScreenErrorCode,
        detail: String,
    ) -> io::Result<()> {
        let error = ScreenWireError::new(code, Some(detail));
        self.frame(HostFrame::Error(error)).await
    }

    /// `CONTROL_STATE` with `blocked`; the next batch that reaches the host
    /// clears it again (D6).
    pub(super) async fn report_block(
        &mut self,
        blocked: crate::screen::core::wire_control::ControlBlock,
    ) -> io::Result<()> {
        self.health.block_reported = Some(blocked);
        self.health.block_shown_until = None;
        // This state names no hold: the next hold is reported again (N4).
        self.health.held_reported = false;
        let state = ControlState {
            holder: self.handle.controller(),
            blocked: Some(blocked),
            held: None,
        };
        self.frame(HostFrame::ControlState(state)).await
    }

    /// E2: a batch the host held back (hidden view, no presented picture of
    /// the current reference yet, a release first) is dropped as before;
    /// the viewer hears it once per hold, while it holds the control. The
    /// state repeats the block it heard last, so a viewer without `held`
    /// (it skips the field) keeps showing that block.
    /// A batch the sink accepted and the platform refused afterwards (an
    /// Android gesture, a Wayland portal call) reaches the viewer as a block
    /// once, while this session holds the control (review 2026-11-19).
    /// Also once a second (`Session::second`, Gemini 3/10): a still screen
    /// may bring no viewer frame for a while.
    pub(in super::super) async fn report_refusal(&mut self) -> io::Result<()> {
        let Some(block) = self
            .pipeline
            .input_sink()
            .and_then(|sink| sink.take_refusal())
        else {
            return Ok(());
        };
        if !self.handle.holds_control() {
            return Ok(());
        }
        self.report_block(block).await?;
        // Z2: no other signal names it (Wayland): it stays shown a while.
        self.health.block_shown_until = Some(Instant::now() + LATE_BLOCK_SHOWN);
        Ok(())
    }

    /// Why input waits now, if it does (E2).
    pub(super) fn hold_reason(&self) -> Option<InputHeld> {
        if !self.visible {
            Some(InputHeld::ViewerHidden)
        } else if self.input_release.is_some() {
            Some(InputHeld::ReleasePending)
        } else if self.input_floor.is_some() || !self.pipeline.input_reference_ready() {
            Some(InputHeld::WaitingForPicture)
        } else {
            None
        }
    }

    /// S4: the viewer heard of a hold whose reason ended: it hears that it
    /// ended (the block it heard last stays named).
    pub(super) async fn end_hold(&mut self) -> io::Result<()> {
        if !self.health.held_reported || self.hold_reason().is_some() {
            return Ok(());
        }
        self.health.held_reported = false;
        let state = ControlState {
            holder: self.handle.controller(),
            blocked: self.health.block_reported,
            held: None,
        };
        self.frame(HostFrame::ControlState(state)).await
    }

    pub(super) async fn report_held(&mut self, held: InputHeld) -> io::Result<()> {
        if self.health.held_reported || !self.handle.holds_control() {
            return Ok(());
        }
        self.health.held_reported = true;
        let state = ControlState {
            holder: self.handle.controller(),
            blocked: self.health.block_reported,
            held: Some(held),
        };
        self.frame(HostFrame::ControlState(state)).await
    }

    /// D2: a batch refused as `control_unavailable` may only have named a
    /// reference the source left after the session last heard of it (the
    /// sinks compare the batch's reference with the source's own, e.g.
    /// `WindowsInput::inject`, and answer that with `control_unavailable`;
    /// the session learns of the source's new reference with the next
    /// picture operation and then holds input until the new reference is
    /// presented, `refresh_input_reference`). Such a batch is dropped
    /// without a block. The refusal is a block once a picture operation
    /// that started after it has ended with the reference unchanged.
    pub(super) fn stale_refusal(
        &mut self,
        blocked: crate::screen::core::wire_control::ControlBlock,
        reference: u64,
    ) -> bool {
        use crate::screen::core::wire_control::ControlBlock;
        if blocked != ControlBlock::ControlUnavailable {
            self.health.unconfirmed = None;
            return false;
        }
        self.pipeline.poll();
        let now = self.pipeline.input_reference_generation();
        let started = self.pipeline.work_started();
        match self.health.unconfirmed {
            Some((before, at)) if before == reference && now == reference => {
                let ended =
                    started > at.saturating_add(1) || (started > at && !self.pipeline.working());
                if ended {
                    self.health.unconfirmed = None;
                }
                !ended
            }
            _ => {
                self.health.unconfirmed = Some((reference, started));
                true
            }
        }
    }
}
