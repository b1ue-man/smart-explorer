//! The full view of the desktop viewer (SCR-M7, U25-3/7): one device, one
//! session, the picture area and the choices of the toolbar carried out.
//! The session opens once the profiles are read and the target still names
//! the device it was made for (a Direct contact whose device changed is
//! refused). „Steuern“ needs a session in the mode `control`: switching it
//! on in a plain viewing session opens a new one, for which the host asks.
use std::time::{Duration, Instant};

use super::screen_devices::{entries, Devices};
use super::screen_pool::{full_request, full_set_mode, ViewOpen};
use super::screen_prefs::ViewerPrefs;
use super::screen_session::{Backend, ViewerSession};
use super::screen_target::ScreenTarget;
use super::screen_texts::state_text;
use super::screen_toolbar_ui::ToolbarAction;
use super::screen_view_widget::ViewWidget;
use crate::screen::core::input_model::{InputEvent, SystemAction};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::{ControlAction, ControlRequest, ViewerFrame};
use crate::screen::core::wire_hello::SessionMode;

/// A window resize is passed on after this long without a newer size.
const RESIZE_PAUSE: Duration = Duration::from_millis(400);

/// What both modes of the window share.
pub(crate) struct Common {
    pub backend: Backend,
    pub devices: Devices,
    pub prefs: ViewerPrefs,
    /// The GPU path is installed.
    pub gpu: bool,
    pub stats_on: bool,
    pub low_latency_changed: bool,
}

pub(crate) struct ViewState {
    pub target: ScreenTarget,
    pub session: Option<ViewerSession>,
    pub widget: ViewWidget,
    pub source: Option<SourceId>,
    pub control_on: bool,
    /// The target was refused (`Das Gerät hat sich geändert …`).
    pub refused: Option<String>,
    sent_display: FrameSize,
    resized_at: Instant,
}

impl ViewState {
    pub(crate) fn new(target: ScreenTarget, gpu: bool) -> Self {
        Self {
            source: target.source.clone(),
            control_on: target.control,
            target,
            session: None,
            widget: ViewWidget::new(gpu),
            refused: None,
            sent_display: FrameSize::new(0, 0),
            resized_at: Instant::now(),
        }
    }

    /// A state that continues with a session that already runs.
    pub(crate) fn with_session(
        target: ScreenTarget,
        session: ViewerSession,
        display: FrameSize,
        gpu: bool,
    ) -> Self {
        let mut state = Self::new(target, gpu);
        state.session = Some(session);
        state.sent_display = display;
        state
    }

    pub(crate) fn open_options(common: &Common, display: FrameSize) -> ViewOpen {
        ViewOpen {
            display,
            prefs: common.prefs.clone(),
            prefer_yuv: common.gpu,
        }
    }

    /// Opens the session when the profiles are read and name the device.
    pub(crate) fn ensure_session(&mut self, common: &Common, display: FrameSize) {
        if self.session.is_some() || self.refused.is_some() {
            return;
        }
        let checked = {
            let state = common.devices.lock();
            state
                .profiles
                .as_ref()
                .map(|profiles| self.target.check_profiles(profiles))
        };
        match checked {
            None => {}
            Some(Err(message)) => self.refused = Some(message),
            Some(Ok(())) => self.open(common, display),
        }
    }

    fn open(&mut self, common: &Common, display: FrameSize) {
        let request = full_request(
            &common.backend.offers,
            self.source.clone(),
            display,
            self.control_on,
        );
        self.session = Some(ViewerSession::open(
            &common.backend,
            self.target.clone(),
            request,
            self.control_on,
            common.gpu,
            0,
        ));
        self.sent_display = display;
        self.widget = ViewWidget::new(common.gpu);
    }

    /// Ends the session and opens a new one to the same target.
    pub(crate) fn reconnect(&mut self, common: &Common, display: FrameSize) {
        self.end_session();
        self.refused = None;
        // The profiles are checked again: a changed device stays refused.
        self.ensure_session(common, display);
    }

    pub(crate) fn end_session(&mut self) {
        if let Some(session) = self.session.take() {
            self.widget.release_input(&session);
            session.close();
        }
    }

    /// The device's name as the window shows it.
    pub(crate) fn device_name(&self, common: &Common) -> String {
        common
            .devices
            .lock()
            .profiles
            .as_ref()
            .and_then(|profiles| self.target.device_name(profiles))
            .unwrap_or_else(|| "Gerät".into())
    }

    /// The text of the status line: the state, or why the session ended.
    pub(crate) fn status_text(&self) -> String {
        if let Some(message) = &self.refused {
            return message.clone();
        }
        let Some(session) = &self.session else {
            return state_text(SessionState::Connecting, None);
        };
        let info = session.info();
        if let Some(end) = &info.end {
            return end.text.clone();
        }
        let text = state_text(info.state, info.reason.as_deref());
        match (&info.notice, text.is_empty()) {
            (Some(notice), true) => notice.clone(),
            (Some(notice), false) => format!("{text} – {notice}"),
            (None, _) => text,
        }
    }

    /// The session ended or the target was refused: „Erneut verbinden“ helps.
    pub(crate) fn is_over(&self) -> bool {
        self.refused.is_some() || self.session.as_ref().is_some_and(ViewerSession::is_ended)
    }

    /// Passes a resized window on to the host, once the size rests.
    pub(crate) fn follow_size(&mut self, display: FrameSize, prefs: &ViewerPrefs) {
        let Some(session) = &self.session else {
            return;
        };
        let moved = |old: u32, new: u32| old.abs_diff(new) >= (old / 8).max(64);
        let changed = moved(self.sent_display.width, display.width)
            || moved(self.sent_display.height, display.height);
        if !changed {
            self.resized_at = Instant::now();
            return;
        }
        if self.resized_at.elapsed() >= RESIZE_PAUSE && session.info().accepted > 0 {
            session.send(ViewerFrame::SetMode(full_set_mode(None, display, prefs)));
            self.sent_display = display;
            self.resized_at = Instant::now();
        }
    }

    fn send_keys(&self, events: Vec<InputEvent>) {
        if let (Some(session), true) = (&self.session, self.control_on) {
            if !session.input_ready() { return; }
            session.send(ViewerFrame::Input(events));
        }
    }

    fn take_control(&mut self, common: &Common, display: FrameSize, on: bool) {
        self.control_on = on;
        self.target.control = on;
        let Some(session) = &self.session else {
            return;
        };
        if on {
            session.info().control_denied = false;
            if session.mode == SessionMode::Control {
                let take = ControlRequest {
                    action: ControlAction::Take,
                };
                session.send(ViewerFrame::ControlRequest(take));
            } else {
                // Only a session in the mode `control` may take the control.
                self.reconnect(common, display);
            }
        } else {
            self.widget.release_input(session);
            let release = ControlRequest {
                action: ControlAction::Release,
            };
            session.send(ViewerFrame::ControlRequest(release));
        }
    }

    /// Carries out a choice of the toolbar that concerns this view; the
    /// others (device, all devices, full screen) are the app's.
    pub(crate) fn apply(
        &mut self,
        action: &ToolbarAction,
        common: &mut Common,
        display: FrameSize,
    ) {
        match action {
            ToolbarAction::Source(source) => {
                self.source = source.clone();
                if let Some(session) = &self.session {
                    self.widget.source_changed(session);
                    let mode = full_set_mode(source.clone(), display, &common.prefs);
                    session.send(ViewerFrame::SetMode(mode));
                }
            }
            ToolbarAction::RefreshWindows => {
                if let Some(session) = &self.session {
                    session.send(ViewerFrame::ListWindows);
                }
            }
            ToolbarAction::Control(on) => self.take_control(common, display, *on),
            ToolbarAction::Sharpness(_) | ToolbarAction::Lossless(_) => {
                if let Some(session) = &self.session {
                    let mode = full_set_mode(None, display, &common.prefs);
                    session.send(ViewerFrame::SetMode(mode));
                }
            }
            ToolbarAction::Key(usage) => self.send_keys(vec![
                InputEvent::Key {
                    hid_usage: *usage,
                    down: true,
                },
                InputEvent::Key {
                    hid_usage: *usage,
                    down: false,
                },
            ]),
            ToolbarAction::System(action) => self.send_system(*action),
            _ => {}
        }
    }

    fn send_system(&self, action: SystemAction) {
        self.send_keys(vec![InputEvent::Action { action }]);
    }
}

impl Drop for ViewState {
    fn drop(&mut self) {
        self.end_session();
    }
}

/// The devices of the profiles for the device choice.
pub(crate) fn device_list(devices: &Devices) -> Vec<super::screen_devices::DeviceEntry> {
    let state = devices.lock();
    state.profiles.as_ref().map_or_else(Vec::new, |profiles| {
        entries(profiles, crate::share::core_now_secs())
    })
}
