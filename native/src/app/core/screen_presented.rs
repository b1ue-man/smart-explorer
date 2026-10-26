//! Proof carried from one decoded frame into the renderer of its active session.
use std::sync::Arc;
use eframe::{egui, egui_wgpu::{self, wgpu}};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PresentedFrame {
    pub epoch: u64,
    pub gop_id: u64,
    pub frame_id: u64,
}
#[derive(Default)]
pub(crate) struct PresentationState {
    pub epoch: u64,
    pub gop_id: u64,
    pub closed: bool,
    pub last: Option<PresentedFrame>,
}
impl PresentationState {
    pub(crate) fn invalidate(&mut self) {
        self.epoch = self.epoch.saturating_add(1);
        self.last = None;
    }
    pub(crate) fn matches(&self, frame: PresentedFrame) -> bool {
        !self.closed && frame.epoch == self.epoch && frame.gop_id == self.gop_id
    }
}
pub(crate) type Presented = Arc<dyn Fn() + Send + Sync>;

/// Runs after the CPU image mesh was taken into the renderer's render pass.
/// A decoded slot or a pending resampling job never reaches this callback.
struct CpuPresented(Presented);
impl egui_wgpu::CallbackTrait for CpuPresented {
    fn paint(&self, _info: egui::PaintCallbackInfo,
        _pass: &mut wgpu::RenderPass<'static>, _resources: &egui_wgpu::CallbackResources) {
        (self.0)();
    }
}
pub(crate) fn cpu_callback(rect: egui::Rect, presented: Presented, wgpu: bool) -> egui::PaintCallback {
    if wgpu { egui_wgpu::Callback::new_paint_callback(rect, CpuPresented(presented)) }
    else { egui::PaintCallback { rect, callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_, _| presented())) } }
}

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
        PresentedFrame { epoch: state.epoch, gop_id, frame_id }
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

    pub(super) fn can_present(&self, frame: PresentedFrame) -> bool {
        lock(&self.presentation).matches(frame) && lock(&self.resume).can_present(frame.frame_id)
            && self.display_active()
    }

    pub(super) fn presented(&self, frame: PresentedFrame, send: impl FnOnce() -> bool) -> bool {
        let mut state = lock(&self.presentation);
        if !state.matches(frame) || !self.display_active() { return false; }
        let mut resume = lock(&self.resume);
        if !resume.can_present(frame.frame_id) { return false; }
        if state.last.is_some_and(|last| last.gop_id == frame.gop_id && last.frame_id >= frame.frame_id) { return false; }
        if !send() { return false; }
        state.last = Some(frame);
        resume.complete(frame.frame_id);
        true
    }

    pub(crate) fn stop_presentation(&self) {
        lock(&self.presentation).closed = true;
        self.invalidate_presentation();
    }

    /// Reference loss blocks input while retaining an already acknowledged visibility floor.
    pub(crate) fn cancel_presentation(&self) {
        lock(&self.presentation).invalidate();
        lock(&self.resume).block();
        lock(&self.picture).take();
    }
}
