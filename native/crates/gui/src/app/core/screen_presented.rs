//! Proof carried from one decoded frame into the renderer of its active session.
use std::sync::Arc;

/// Allocation identity is local; it never changes the wire protocol.
#[derive(Clone, Debug, Default)]
pub struct SessionToken(Arc<()>);
impl PartialEq for SessionToken {
    fn eq(&self, rhs: &Self) -> bool {
        Arc::ptr_eq(&self.0, &rhs.0)
    }
}
impl Eq for SessionToken {}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresentedFrame {
    pub session: SessionToken,
    pub validity: super::screen_render_upload::Generation,
    pub login: Option<Arc<crate::login::FrameToken>>,
    pub epoch: u64,
    pub gop_id: u64,
    pub frame_id: u64,
}
impl PresentedFrame {
    pub fn active(&self) -> bool {
        self.validity.active()
    }
    /// Pixels of `rhs` may replace these in place: the same session and either
    /// the same still-active validity (a new picture group of the same stream
    /// keeps it, `Shared::begin_frame`) or the same group and receipt epoch.
    pub fn same_source(&self, rhs: &Self) -> bool {
        self.session == rhs.session
            && self.active()
            && (self.validity == rhs.validity
                || (self.gop_id == rhs.gop_id && self.epoch == rhs.epoch))
    }
}
#[derive(Default)]
pub struct PresentationState {
    pub session: SessionToken,
    pub validity: super::screen_render_upload::Generation,
    pub epoch: u64,
    pub gop_id: u64,
    pub closed: bool,
    pub last: Option<PresentedFrame>,
}
impl PresentationState {
    pub fn invalidate(&mut self) {
        self.validity.revoke();
        self.validity = super::screen_render_upload::Generation::default();
        self.suspend_receipts();
    }
    fn suspend_receipts(&mut self) {
        self.last = None;
        if let Some(epoch) = self.epoch.checked_add(1) {
            self.epoch = epoch;
        } else {
            self.validity.revoke();
            self.validity = super::screen_render_upload::Generation::default();
        }
    }

    pub fn matches(&self, frame: &PresentedFrame) -> bool {
        !self.closed
            && frame.validity == self.validity
            && frame.active()
            && frame.session == self.session
            && frame.epoch == self.epoch
            && frame.gop_id == self.gop_id
    }
}
pub type Presented = Arc<dyn Fn() + Send + Sync>;

use super::screen_session::Shared;
use std::sync::{Mutex, MutexGuard, PoisonError};
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
impl Shared {
    /// Captured before decoding, so cancellation cannot bless an in-flight old image.
    pub fn begin_frame(&self, gop_id: u64, frame_id: u64) -> PresentedFrame {
        let mut state = lock(&self.presentation);
        if !state.validity.active() {
            state.gop_id = gop_id;
            state.invalidate();
            lock(&self.resume).block();
        } else if state.gop_id != gop_id {
            // A new picture group (key frame after a stream reset, a decode
            // error or a key frame request, a continuation) ends the receipts
            // of the old one, but the last shown pixels stay until its first
            // picture replaces them. Revoking them here blanked the picture
            // from the group's first record until its key frame was
            // transferred, decoded and uploaded: a black flash per stream
            // reset. Input goes on (fix 2026-11-19): the source, its size on
            // screen and the host's visibility floor did not change, and
            // blocking it here released every held key and button with each
            // new group (a drag broke off, typing was lost); a real change of
            // the reference invalidates the presentation and blocks it there.
            state.gop_id = gop_id;
            state.suspend_receipts();
        }
        PresentedFrame {
            session: state.session.clone(),
            validity: state.validity.clone(),
            login: None,
            epoch: state.epoch,
            gop_id,
            frame_id,
        }
    }

    pub fn invalidate_presentation(&self) {
        lock(&self.presentation).invalidate();
        lock(&self.resume).invalidate();
        lock(&self.picture).take();
    }

    pub(super) fn modern(&self) -> bool {
        self.info()
            .hello
            .as_ref()
            .and_then(|hello| hello.frame_contract)
            .is_some_and(|contract| {
                contract.version == crate::screen::core::frame_budget::FRAME_PROTOCOL
            })
    }

    fn display_active(&self) -> bool {
        let info = self.info();
        !info.is_ended()
            && matches!(
                info.state,
                crate::screen::core::session_state::SessionState::Starting
                    | crate::screen::core::session_state::SessionState::Streaming
            )
    }

    /// `frame` belongs to the current presentation but waits for the
    /// visibility acknowledgement (`ResumeGate::awaiting_floor`).
    pub(super) fn awaiting_floor(&self, frame: &PresentedFrame) -> bool {
        lock(&self.presentation).matches(frame)
            && lock(&self.resume).awaiting_floor()
            && self.display_active()
    }

    /// Puts a picture that could not be shown yet back, unless a newer one
    /// arrived meanwhile; the acknowledgement's wake shows it.
    pub(super) fn keep_picture(&self, picture: super::screen_session::Picture) {
        let presentation = lock(&self.presentation);
        if presentation.matches(&picture.presented) {
            let mut slot = lock(&self.picture);
            if slot.is_none() {
                *slot = Some(picture);
            }
        }
    }

    pub(super) fn can_present(&self, frame: &PresentedFrame) -> bool {
        lock(&self.presentation).matches(frame)
            && lock(&self.resume).can_present(frame.frame_id)
            && self.display_active()
    }

    pub(super) fn presented(&self, frame: PresentedFrame, send: impl FnOnce() -> bool) -> bool {
        let mut state = lock(&self.presentation);
        if !state.matches(&frame) || !self.display_active() {
            return false;
        }
        let mut resume = lock(&self.resume);
        if !resume.can_present(frame.frame_id) {
            return false;
        }
        if state
            .last
            .as_ref()
            .is_some_and(|last| last.gop_id == frame.gop_id && last.frame_id >= frame.frame_id)
        {
            return false;
        }
        if !send() {
            return false;
        }
        // A new picture was presented (statistics, `screen_flow.rs`).
        self.info().latency.flow.presented();
        resume.complete(frame.frame_id);
        state.last = Some(frame);
        true
    }

    pub fn stop_presentation(&self) {
        lock(&self.presentation).closed = true;
        self.invalidate_presentation();
        let memory = lock(&self.memory).take();
        self.retirement.watch_memory(&memory);
        self.retirement.close();
        super::screen_render_owner::notify_all();
    }

    /// A broken or undecodable group: the shown pixels stay without their
    /// receipt until the next group replaces them; input goes on, since the
    /// reference on screen did not change (fix 2026-11-19, `begin_frame`).
    pub fn pause_receipts(&self) {
        lock(&self.presentation).suspend_receipts();
        lock(&self.picture).take();
    }

    /// A paused stream retains its already valid last pixels for dimming, never their receipt.
    pub fn suspend_presentation(&self) {
        lock(&self.presentation).suspend_receipts();
        lock(&self.resume).block();
        lock(&self.picture).take();
    }

    /// Reference loss blocks input while retaining an already acknowledged visibility floor.
    pub fn cancel_presentation(&self) {
        lock(&self.presentation).invalidate();
        lock(&self.resume).block();
        lock(&self.picture).take();
    }
}

impl super::screen_session::ViewerSession {
    /// Queues the negotiated visibility report. Old peers retain their wire contract.
    pub fn set_visibility(&self, mut report: crate::screen::core::visibility::ViewerVisibility) {
        report.geometry_generation = lock(&self.shared.visibility)
            .as_ref()
            .map_or(1, |(previous, _)| {
                previous.geometry_generation.saturating_add(1)
            });
        let modern = self.shared.modern();
        // An older host (no visibility contract, Codex 10) hears nothing of
        // the report and sends no fresh picture for it: a view that stays
        // visible keeps its picture and input, and one shown again after it
        // was hidden asks for a key frame (a still screen sends none).
        let was_hidden = lock(&self.shared.visibility)
            .as_ref()
            .is_some_and(|(previous, _)| previous.hidden());
        if modern || report.hidden() {
            self.shared.invalidate_presentation();
        }
        lock(&self.shared.resume).report(&report, modern);
        *lock(&self.shared.visibility) = Some((report.clone(), modern));
        if modern {
            self.send(crate::screen::core::wire_control::ViewerFrame::Visibility(
                report,
            ));
        } else if was_hidden && !report.hidden() {
            let request = crate::screen::core::wire_control::KeyframeRequest {
                reason: crate::screen::core::wire_control::KeyframeReason::Reset,
                gop_id: None,
            };
            self.send(crate::screen::core::wire_control::ViewerFrame::KeyframeRequest(request));
        }
    }

    /// Replays the latest facts once HELLO proves the new frame contract.
    pub fn refresh_visibility(&self) {
        let modern = self.shared.modern();
        let report = lock(&self.shared.visibility)
            .as_ref()
            .filter(|(_, sent_modern)| modern && !sent_modern)
            .map(|(report, _)| report.clone());
        if let Some(report) = report {
            self.set_visibility(report);
        }
    }

    /// A picture that matches the presentation but arrived before the host's
    /// visibility acknowledgement goes back into the slot instead of being
    /// dropped: a host with an unchanged screen sends no next picture, and
    /// the view stayed black with input blocked.
    pub fn keep_if_awaiting_floor(&self, picture: super::screen_session::Picture) {
        if self.shared.awaiting_floor(&picture.presented) {
            self.shared.keep_picture(picture);
        } else {
            // Not presentable under the host's visibility floor (statistics).
            let floor = super::screen_chain_window::flow::Dropped::Floor;
            self.shared.update(|info| info.latency.flow.dropped(floor));
        }
    }
}
