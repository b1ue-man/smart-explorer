//! The source of the Windows host (SCR-M4, U16-7 and U16-9): one monitor
//! or one window, captured with Graphics Capture or, for a monitor, with
//! the DXGI duplication. Before every capture the states that pause it are
//! checked in a fixed order (`state_before_capture`); a capture that comes
//! out of a pause on the secure desktop or in an inactive session starts
//! its capture session anew, because both APIs can be invalid after the
//! desktop changed.
use std::io;
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

use super::capture_dxgi::DxgiCapture;
use super::capture_wgc::{self, Target, WgcCapture};
use super::{d3d_device, monitors, secure_desktop};
use crate::screen::core::host_backend::{ScreenSource, SourceFrame, SourceRefusal};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::SourceId;

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
    fn frame_limits(&self)->crate::screen::core::frame_budget::FrameLimits {crate::screen::core::frame_budget::FrameLimits::LEGACY}
    fn reference_generation(&self)->u64 {0}
    /// The newest picture; `Unchanged` when none arrived and the caller
    /// needs none, the last picture again with `keyframe`, `Paused(Starting)`
    /// when there is none yet after `FIRST_FRAME_WAIT`.
    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame>;
    /// Creates the capture session anew.
    fn reopen(&mut self) -> io::Result<()>;
    fn source_available(&self) -> bool {true}
    fn set_capture_active(&mut self,_active:bool) -> io::Result<()> {Ok(())}
}

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
}

fn refusal(reason: &str) -> SourceRefusal {
    SourceRefusal::new(reason)
}

impl WindowsSource {
    /// Opens `source` (a monitor or a window; the test picture is not
    /// handled here). Graphics Capture is used when the system has it; a
    /// monitor falls back to the DXGI duplication once, a window has no
    /// fallback.
    pub(super) fn open(source: &SourceId) -> Result<Self, SourceRefusal> {
        monitors::ensure_dpi_aware();
        super::mf_common::ensure_com();
        if !secure_desktop::session_active() {
            return Err(refusal(SESSION_INACTIVE));
        }
        match source {
            SourceId::Monitor(id) => Self::open_monitor(id),
            SourceId::Window(text) => Self::open_window(text),
            SourceId::Synthetic => Err(refusal(CAPTURE_UNAVAILABLE)),
        }
    }

    fn open_monitor(id: &str) -> Result<Self, SourceRefusal> {
        let entry = monitors::find(id).ok_or_else(|| refusal(MONITOR_NOT_FOUND))?;
        let wgc = capture_wgc::available()
            .then(|| {
                d3d_device::create()
                    .and_then(|devices| WgcCapture::open(devices, Target::Monitor(id.to_string())))
                    .ok()
            })
            .flatten();
        let grabber: Box<dyn Grabber> = match wgc {
            Some(wgc) => Box::new(wgc),
            None => Box::new(
                DxgiCapture::open(&entry.device).map_err(|_| refusal(CAPTURE_UNAVAILABLE))?,
            ),
        };
        Ok(Self::with(grabber, SystemProbe { window: 0 }, false))
    }

    fn open_window(text: &str) -> Result<Self, SourceRefusal> {
        let handle: usize = text.parse().map_err(|_| refusal(WINDOW_NOT_FOUND))?;
        let mut probe = SystemProbe { window: handle };
        if !probe.window_exists() {
            return Err(refusal(WINDOW_NOT_FOUND));
        }
        if !capture_wgc::available() {
            return Err(refusal(WINDOW_CAPTURE_UNSUPPORTED));
        }
        let devices = d3d_device::create().map_err(|_| refusal(CAPTURE_UNAVAILABLE))?;
        let wgc = WgcCapture::open(devices, Target::Window(handle))
            .map_err(|_| refusal(CAPTURE_UNAVAILABLE))?;
        Ok(Self::with(Box::new(wgc), probe, true))
    }

    fn with(
        grabber: Box<dyn Grabber>,
        probe: impl StateProbe + Send + 'static,
        window: bool,
    ) -> Self {
        Self {
            grabber,
            probe: Box::new(probe),
            window,
            renew: false,
            capture_active: true,
        }
    }
}

impl ScreenSource for WindowsSource {
    fn reference_generation(&self)->u64 {self.grabber.reference_generation()}
    fn frame_limits(&self)->crate::screen::core::frame_budget::FrameLimits {self.grabber.frame_limits()}
    fn set_capture_active(&mut self,active:bool) -> io::Result<()> {
        self.capture_active=false;
        if !self.grabber.source_available() {return Err(io::Error::new(io::ErrorKind::NotFound,"original capture source unavailable"));}
        if active && state_before_capture(self.window,self.probe.as_mut())?.is_some() {self.renew=true; self.capture_active=true; return Ok(());}
        self.grabber.set_capture_active(active)?; self.capture_active=active; Ok(())
    }
    fn size(&self) -> FrameSize {
        self.grabber.size()
    }

    fn capture(&mut self, _frame_id: u64, keyframe: bool) -> io::Result<SourceFrame> {
        if !self.capture_active {return Ok(SourceFrame::Unchanged);}
        if let Some(state) = state_before_capture(self.window, self.probe.as_mut())? {
            self.renew |= matches!(
                state,
                SessionState::SecureDesktop | SessionState::SessionInactive
            );
            return Ok(SourceFrame::Paused(state));
        }
        if self.renew {
            self.grabber.reopen()?;
            self.renew = false;
        }
        self.grabber.capture(keyframe)
    }
}

#[cfg(test)]
#[path = "capture_state_tests.rs"]
mod tests;
