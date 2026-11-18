//! The source of the Windows host (SCR-M4, U16-7 and U16-9): one monitor
//! or one window, captured with Graphics Capture or, for a monitor, with
//! the DXGI duplication. Before every capture the states that pause it are
//! checked in a fixed order (`state_before_capture`); a capture that comes
//! out of a pause on the secure desktop or in an inactive session starts
//! its capture session anew, because both APIs can be invalid after the
//! desktop changed.
//!
//! Plan 2026-11-14 bildkette, BK2b: a monitor is captured with Desktop
//! Duplication first and Graphics Capture as its fallback (`capture_rank`,
//! `fallback_cause`, B-12); an activated source captures on its own thread,
//! woken by the compositor (`capture_thread.rs`, B-10); the session's capture
//! path and rate limit are kept in `CaptureReport`.
//!
//! BK3b: a picture may stay on the GPU (`capture_picture`); `capture` gives
//! CPU pixels (merge W1: exact BGRA). Input has its own way (`WindowsInput`).
use std::io;
use std::sync::Arc;
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

use super::secure_desktop;
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::capture_picture::SourcePicture;
use crate::screen::core::host_backend::{ScreenInputSink, ScreenSource, SourceFrame};
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_control::ControlBlock;

/// How long the first picture of a capture may take: Graphics Capture
/// delivers it within one display refresh, a few multiples of that are
/// well below the 10 s the daemon allows a host process per picture.
pub(super) const FIRST_FRAME_WAIT: Duration = Duration::from_secs(1);

/// Reasons of `no_display` for a source that does not open.
pub(super) const MONITOR_NOT_FOUND: &str = "monitor_not_found";
pub(super) const WINDOW_NOT_FOUND: &str = "window_not_found";
pub(super) const SESSION_INACTIVE: &str = "session_inactive";
pub(super) const CAPTURE_UNAVAILABLE: &str = "capture_unavailable";
pub(super) const WINDOW_CAPTURE_UNSUPPORTED: &str = "window_capture_unsupported";

/// A capture API behind a `WindowsSource`.
pub(super) trait Grabber: Send {
    /// Size of the pictures `capture` delivers.
    fn size(&self) -> FrameSize;
    fn capture_grant(&self) -> Option<CaptureGrant> {
        None
    }
    fn frame_limits(&self) -> crate::screen::core::frame_budget::FrameLimits {
        crate::screen::core::frame_budget::FrameLimits::LEGACY
    }
    fn reference_generation(&self) -> u64 {
        0
    }
    /// The newest picture; `Unchanged` when none arrived and the caller
    /// needs none, the last picture again with `keyframe`, `Paused(Starting)`
    /// when there is none yet after `FIRST_FRAME_WAIT`.
    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame>;
    /// Like `capture`, but waits up to `wait` for the compositor's next
    /// picture (B-10). `release` runs right before a picture is read back and
    /// lets the capture thread drop a superseded picture first; it answers
    /// whether the previous picture was handed out (otherwise this picture's
    /// regions would describe a change against a picture nobody saw).
    fn capture_waiting(
        &mut self,
        keyframe: bool,
        _wait: Duration,
        _release: &mut dyn FnMut() -> bool,
    ) -> io::Result<SourceFrame> {
        self.capture(keyframe)
    }
    /// Like `capture`, but the picture may stay on the GPU (BK3b).
    fn capture_picture(&mut self, keyframe: bool) -> io::Result<SourcePicture> {
        self.capture(keyframe).map(SourcePicture::from)
    }
    /// CPU pixels of `picture`; merge W1: the threaded grabber captures a
    /// GPU picture anew as exact BGRA (still images), others read it back.
    fn exact_frame(&mut self, picture: SourcePicture) -> io::Result<SourceFrame> {
        picture.into_source_frame()
    }
    /// Like `capture_waiting`, but the picture may stay on the GPU (BK3b).
    fn capture_waiting_picture(
        &mut self,
        keyframe: bool,
        wait: Duration,
        release: &mut dyn FnMut() -> bool,
    ) -> io::Result<SourcePicture> {
        self.capture_waiting(keyframe, wait, release)
            .map(SourcePicture::from)
    }
    /// Reads the current reference generation without the grabber (input).
    fn generation_reader(&self) -> Option<GenerationReader> {
        None
    }
    /// Creates the capture session anew.
    fn reopen(&mut self) -> io::Result<()>;
    fn source_available(&self) -> bool {
        true
    }
    fn set_capture_active(&mut self, _active: bool) -> io::Result<()> {
        Ok(())
    }
    /// The frame interval the pipeline asks at (B-11); false when the capture
    /// cannot limit its delivery to it (`capture_rate_capped`).
    fn set_frame_interval(&mut self, _interval: Duration) -> bool {
        true
    }
    /// The API this grabber captures with.
    fn path(&self) -> Option<CapturePath> {
        None
    }
    /// Why Desktop Duplication should give way to Graphics Capture now.
    fn fallback_cause(&self) -> Option<FallbackCause> {
        None
    }
    /// The failed step and result code behind `fallback_cause` (P9).
    fn fallback_detail(&self) -> Option<crate::screen::core::capture_fallback::FallbackDetail> {
        None
    }
}

/// Reads a grabber's reference generation from any thread.
pub(super) type GenerationReader = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Input of a Windows source on its own way (BK3b, plan B-28): the same
/// check as `ScreenSource::inject_input` (the viewer's reference generation
/// is the source's) and the same `SendInput`, without the source.
struct WindowsInput {
    generation: GenerationReader,
}

impl ScreenInputSink for WindowsInput {
    fn inject(
        &self,
        events: &[InputEvent],
        reference_generation: u64,
        _presented: bool,
    ) -> Result<(), ControlBlock> {
        if reference_generation != (self.generation)() {
            return Err(ControlBlock::ControlUnavailable);
        }
        super::input_send::inject(events)
    }
    fn release(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        super::input_send::release(events)
    }
}

/// The capture API of a source (B-12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CapturePath {
    DesktopDuplication,
    GraphicsCapture,
}

impl CapturePath {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::DesktopDuplication => "desktop_duplication",
            Self::GraphicsCapture => "graphics_capture",
        }
    }
}

/// The order a source tries the capture APIs in: a monitor with Desktop
/// Duplication first (separate pointer, dirty and move rectangles, no yellow
/// border), Graphics Capture after it; a window only with Graphics Capture.
pub(super) fn capture_rank(monitor: bool, graphics_capture: bool) -> Vec<CapturePath> {
    let mut rank = Vec::with_capacity(2);
    if monitor {
        rank.push(CapturePath::DesktopDuplication);
    }
    if graphics_capture {
        rank.push(CapturePath::GraphicsCapture);
    }
    rank
}

#[path = "capture_fallback_report.rs"]
mod fallback_report;
#[cfg_attr(not(test), allow(unused_imports))]
pub(super) use fallback_report::{
    detail_of, fallback_bits, fallback_cause, log_failure, rebuild_fallback, report, result_code,
    update_report, CaptureReport, FallbackCause, FALLBACK_AFTER,
};

/// The questions that decide whether a capture pauses.
pub(super) trait StateProbe {
    fn session_active(&mut self) -> bool;
    fn on_secure_desktop(&mut self) -> bool;
    fn window_exists(&mut self) -> bool;
    fn window_minimized(&mut self) -> bool;
}

/// The state that pauses the capture now, in this order: inactive session,
/// secure desktop, and for a window one that is gone (an error: the session
/// ends and the viewer picks another source) or minimized (asked for every
/// capture, never derived from missing pictures).
pub(super) fn state_before_capture(
    window: bool,
    probe: &mut dyn StateProbe,
) -> io::Result<Option<SessionState>> {
    if !probe.session_active() {
        return Ok(Some(SessionState::SessionInactive));
    }
    if probe.on_secure_desktop() {
        return Ok(Some(SessionState::SecureDesktop));
    }
    if window {
        if !probe.window_exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Das Fenster existiert nicht mehr",
            ));
        }
        if probe.window_minimized() {
            return Ok(Some(SessionState::WindowMinimized));
        }
    }
    Ok(None)
}

/// The probe of this system; `window` is the handle value of a window source.
struct SystemProbe {
    window: usize,
}

impl SystemProbe {
    fn hwnd(&self) -> HWND {
        HWND(self.window as *mut _)
    }
}

impl StateProbe for SystemProbe {
    fn session_active(&mut self) -> bool {
        secure_desktop::session_active()
    }

    fn on_secure_desktop(&mut self) -> bool {
        secure_desktop::on_secure_desktop()
    }

    fn window_exists(&mut self) -> bool {
        // SAFETY: a plain query on a window handle value.
        unsafe { IsWindow(self.hwnd()).as_bool() }
    }

    fn window_minimized(&mut self) -> bool {
        // SAFETY: a plain query on a window handle value.
        unsafe { IsIconic(self.hwnd()).as_bool() }
    }
}

/// A monitor or a window of this Windows session.
pub(super) struct WindowsSource {
    grabber: Box<dyn Grabber>,
    probe: Box<dyn StateProbe + Send>,
    window: bool,
    /// The last capture paused on the secure desktop or in an inactive
    /// session: the capture session is made anew before the next picture.
    renew: bool,
    capture_active: bool,
    input: Option<Arc<WindowsInput>>,
}

impl WindowsSource {
    /// The source of an activated grabber, which captures on its own thread
    /// from now on (B-10); without a thread it captures per request as before.
    pub(super) fn activated(grabber: Box<dyn Grabber>, window: Option<usize>) -> Self {
        let handle = window.unwrap_or(0);
        let grabber = thread::start(
            grabber,
            Box::new(SystemProbe { window: handle }),
            window.is_some(),
        );
        Self::with(grabber, SystemProbe { window: handle }, window.is_some())
    }

    fn with(
        grabber: Box<dyn Grabber>,
        probe: impl StateProbe + Send + 'static,
        window: bool,
    ) -> Self {
        let input = grabber
            .generation_reader()
            .map(|generation| Arc::new(WindowsInput { generation }));
        Self {
            grabber,
            probe: Box::new(probe),
            window,
            renew: false,
            capture_active: true,
            input,
        }
    }
}

impl ScreenSource for WindowsSource {
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.grabber.capture_grant()
    }
    fn reference_generation(&self) -> u64 {
        self.grabber.reference_generation()
    }
    fn input_sink(&self) -> Option<Arc<dyn ScreenInputSink>> {
        self.input
            .clone()
            .map(|input| input as Arc<dyn ScreenInputSink>)
    }
    fn frame_limits(&self) -> crate::screen::core::frame_budget::FrameLimits {
        self.grabber.frame_limits()
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.capture_active = false;
        if !self.grabber.source_available() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "original capture source unavailable",
            ));
        }
        if active && state_before_capture(self.window, self.probe.as_mut())?.is_some() {
            self.renew = true;
            self.capture_active = true;
            return Ok(());
        }
        self.grabber.set_capture_active(active)?;
        self.capture_active = active;
        Ok(())
    }
    fn size(&self) -> FrameSize {
        self.grabber.size()
    }

    /// CPU pixels (still images): a GPU picture is captured anew exactly.
    fn capture(&mut self, frame_id: u64, keyframe: bool) -> io::Result<SourceFrame> {
        let picture = self.capture_picture(frame_id, keyframe)?;
        self.grabber.exact_frame(picture)
    }

    fn capture_picture(&mut self, _frame_id: u64, keyframe: bool) -> io::Result<SourcePicture> {
        if !self.capture_active {
            return Ok(SourcePicture::Unchanged);
        }
        if let Some(state) = state_before_capture(self.window, self.probe.as_mut())? {
            self.renew |= matches!(
                state,
                SessionState::SecureDesktop | SessionState::SessionInactive
            );
            return Ok(SourcePicture::Paused(state));
        }
        if self.renew {
            self.grabber.reopen()?;
            self.renew = false;
        }
        self.grabber.capture_picture(keyframe)
    }
}

#[path = "capture_thread.rs"]
mod thread;

#[cfg(test)]
#[path = "capture_state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bildkette_capture_task_tests.rs"]
mod bildkette_tests;

#[cfg(test)]
#[path = "bildkette_capture_thread_task_tests.rs"]
mod bildkette_thread_tests;

#[cfg(test)]
#[path = "bildkette_input_task_tests.rs"]
mod bildkette_input_tests;
