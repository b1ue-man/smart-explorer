//! Proof carried from one decoded frame into the renderer of its active session.
use std::sync::Arc;

/// Allocation identity is local; it never changes the wire protocol.
#[derive(Clone, Debug, Default)]
pub(crate) struct SessionToken(Arc<()>);
impl PartialEq for SessionToken { fn eq(&self, rhs:&Self)->bool { Arc::ptr_eq(&self.0,&rhs.0) } }
impl Eq for SessionToken {}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PresentedFrame {
    pub session: SessionToken,
    pub validity:super::screen_render_upload::Generation,
    pub login:Option<Arc<crate::login::FrameToken>>,
    pub epoch: u64,
    pub gop_id: u64,
    pub frame_id: u64,
}
impl PresentedFrame {
    pub(crate) fn active(&self)->bool {self.validity.active()}
    pub(crate) fn same_source(&self,rhs:&Self)->bool {
        self.session==rhs.session&&self.gop_id==rhs.gop_id&&self.active()&&(self.epoch==rhs.epoch||self.validity==rhs.validity)
    }
}
#[derive(Default)]
pub(crate) struct PresentationState {
    pub session: SessionToken,
    pub validity:super::screen_render_upload::Generation,
    pub epoch: u64,
    pub gop_id: u64,
    pub closed: bool,
    pub last: Option<PresentedFrame>,
}
impl PresentationState {
    pub(crate) fn invalidate(&mut self) {
        self.validity.revoke();self.validity=super::screen_render_upload::Generation::default();self.suspend_receipts();
    }
    fn suspend_receipts(&mut self) {self.last=None;if let Some(epoch)=self.epoch.checked_add(1) {self.epoch=epoch;} else {self.validity.revoke();self.validity=super::screen_render_upload::Generation::default();}}

    pub(crate) fn matches(&self, frame: &PresentedFrame) -> bool {
        !self.closed && frame.validity==self.validity && frame.active() && frame.session == self.session && frame.epoch == self.epoch && frame.gop_id == self.gop_id
    }
}
pub(crate) type Presented = Arc<dyn Fn() + Send + Sync>;

use super::screen_session::Shared;
use std::sync::{Mutex, MutexGuard, PoisonError};
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
impl Shared {
    /// Captured before decoding, so cancellation cannot bless an in-flight old image.
    pub(crate) fn begin_frame(&self, gop_id: u64, frame_id: u64) -> PresentedFrame {
        let mut state = lock(&self.presentation);
        if state.gop_id != gop_id {
            state.gop_id = gop_id;
            state.invalidate();
            lock(&self.resume).block();
        }
        PresentedFrame { session:state.session.clone(), validity:state.validity.clone(),login:None, epoch: state.epoch, gop_id, frame_id }
    }

    pub(crate) fn invalidate_presentation(&self) {
        lock(&self.presentation).invalidate();
        lock(&self.resume).invalidate();
        lock(&self.picture).take();
    }

    pub(super) fn modern(&self) -> bool {
        self.info().hello.as_ref().and_then(|hello| hello.frame_contract)
            .is_some_and(|contract| contract.version == crate::screen::core::frame_budget::FRAME_PROTOCOL)
    }

    fn display_active(&self) -> bool {
        let info = self.info();
        !info.is_ended() && matches!(info.state,
            crate::screen::core::session_state::SessionState::Starting |
            crate::screen::core::session_state::SessionState::Streaming)
    }

    pub(super) fn can_present(&self, frame: &PresentedFrame) -> bool {
        lock(&self.presentation).matches(frame) && lock(&self.resume).can_present(frame.frame_id)
            && self.display_active()
    }

    pub(super) fn presented(&self, frame: PresentedFrame, send: impl FnOnce() -> bool) -> bool {
        let mut state = lock(&self.presentation);
        if !state.matches(&frame) || !self.display_active() { return false; }
        let mut resume = lock(&self.resume);
        if !resume.can_present(frame.frame_id) { return false; }
        if state.last.as_ref().is_some_and(|last| last.gop_id == frame.gop_id && last.frame_id >= frame.frame_id) { return false; }
        if !send() { return false; }
        resume.complete(frame.frame_id);
        state.last = Some(frame);
        true
    }

    pub(crate) fn stop_presentation(&self) {
        lock(&self.presentation).closed = true;
        self.invalidate_presentation();
        let memory=lock(&self.memory).take();
        self.retirement.watch_memory(&memory);self.retirement.close();
        super::screen_render_owner::notify_all();
    }

    /// A paused stream retains its already valid last pixels for dimming, never their receipt.
    pub(crate) fn suspend_presentation(&self) {
        lock(&self.presentation).suspend_receipts();lock(&self.resume).block();lock(&self.picture).take();
    }

    /// Reference loss blocks input while retaining an already acknowledged visibility floor.
    pub(crate) fn cancel_presentation(&self) {
        lock(&self.presentation).invalidate();
        lock(&self.resume).block();
        lock(&self.picture).take();
    }
}
