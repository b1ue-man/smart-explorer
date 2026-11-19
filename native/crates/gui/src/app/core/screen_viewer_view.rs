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
pub struct Common {
    pub backend: Backend,
    pub devices: Devices,
    pub prefs: ViewerPrefs,
    /// The GPU path is installed.
    pub gpu: bool,
    pub stats_on: bool,
    pub low_latency_changed: bool,
}

pub struct ViewState {
    pub target: ScreenTarget,
    pub session: Option<ViewerSession>,
    pub widget: ViewWidget,
    pub source: Option<SourceId>,
    pub control_on: bool,
    /// The target was refused (`Das Gerät hat sich geändert …`).
    pub refused: Option<String>,
    sent_display: FrameSize,
    /// The display of the last frame; `resized_at` is when it last changed.
    seen_display: FrameSize,
    resized_at: Instant,
    /// The picture area in physical pixels, measured before the session
    /// opens (`measure`) and kept by `follow_size`; the opening asks the
    /// host for it (G3: it asked for the whole window, the toolbar's height
    /// more, and the first resize sent a new size at once).
    measured: Option<FrameSize>,
    /// The opening waited one frame for `measured`.
    waited: bool,
}

impl ViewState {
    pub fn new(target: ScreenTarget, gpu: bool) -> Self {
        Self {
            source: target.source.clone(),
            control_on: target.control,
            target,
            session: None,
            widget: ViewWidget::new(gpu),
            refused: None,
            sent_display: FrameSize::new(0, 0),
            seen_display: FrameSize::new(0, 0),
            resized_at: Instant::now(),
            measured: None,
            waited: false,
        }
    }

    /// A state that continues with a session that already runs.
    pub fn with_session(
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

    pub fn open_options(common: &Common, display: FrameSize) -> ViewOpen {
        ViewOpen {
            display,
            prefer_yuv: common.gpu,
        }
    }

    /// Opens the session when the profiles are read and name the device.
    pub fn ensure_session(&mut self, common: &Common, display: FrameSize) {
        // The host ended a tile session for a chosen video codec: it opens
        // again with the current source and choice (Codex 11).
        if self
            .session
            .as_ref()
            .is_some_and(ViewerSession::reopen_wanted)
        {
            self.end_session();
        }
        super::screen_audio::follow(self.session.as_ref());
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
            Some(Ok(())) => {
                // One frame for the picture area to be measured; without it
                // (it never came) the window's size as before.
                if self.measured.is_none() && !self.waited {
                    self.waited = true;
                    return;
                }
                let display = self.measured.unwrap_or(display);
                self.open(common, display)
            }
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
        self.widget.reconnect(common.gpu);
    }

    /// Ends the session and opens a new one to the same target.
    pub fn reconnect(&mut self, common: &Common, display: FrameSize) {
        self.end_session();
        self.refused = None;
        // The profiles are checked again: a changed device stays refused.
        self.ensure_session(common, display);
    }

    pub fn end_session(&mut self) {
        super::screen_audio::forget();
        self.widget.retire();
        if let Some(session) = self.session.take() {
            self.widget.release_input(&session);
            session.close();
        }
    }

    /// The device's name as the window shows it.
    pub fn device_name(&self, common: &Common) -> String {
        common
            .devices
            .lock()
            .profiles
            .as_ref()
            .and_then(|profiles| self.target.device_name(profiles))
            .unwrap_or_else(|| "Gerät".into())
    }

    /// The text of the status line: the state, or why the session ended.
    pub fn status_text(&self) -> String {
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
        let mut status = match (&info.notice, text.is_empty()) {
            (Some(notice), true) => notice.clone(),
            (Some(notice), false) => format!("{text} – {notice}"),
            (None, _) => text,
        };
        let mut add = |line: &str| {
            status = match status.is_empty() {
                true => line.to_string(),
                false => format!("{status} – {line}"),
            };
        };
        // Its own line beside the notice (U2).
        if info.rate_ceiling == crate::daemon::RateCeilingState::UnsupportedByHost {
            add(super::screen_texts::CEILING_UNSUPPORTED);
        }
        // E2: input the host holds back, else a block, while it steers; a
        // block repeated in the held state is the one named before.
        if self.control_on {
            if let Some(held) = info.held {
                add(super::screen_texts::held_text(held));
            } else if let Some(block) = info.blocked {
                add(super::screen_texts::blocked_text(block));
            }
        }
        status
    }

    /// The session ended or the target was refused: „Erneut verbinden“ helps.
    pub fn is_over(&self) -> bool {
        self.refused.is_some() || self.session.as_ref().is_some_and(ViewerSession::is_ended)
    }

    /// Notes the picture area while no session draws in it (`measured`).
    pub fn measure(&mut self, ui: &eframe::egui::Ui) {
        let size = ui.available_size() * ui.ctx().pixels_per_point();
        let size = FrameSize::new(
            size.x.round().max(1.0) as u32,
            size.y.round().max(1.0) as u32,
        );
        if self.measured != Some(size) {
            self.measured = Some(size);
            ui.ctx().request_repaint();
        }
    }

    /// Passes a resized window on to the host, once the size rests.
    pub fn follow_size(&mut self, display: FrameSize, prefs: &ViewerPrefs) {
        self.measured = Some(display);
        let Some(session) = &self.session else {
            return;
        };
        // The pause counts from the last change of the size, so a resize that
        // is still being dragged sends nothing (each size costs the host a new
        // encoder and the viewer a new reference picture).
        if display != self.seen_display {
            self.seen_display = display;
            self.resized_at = Instant::now();
            return;
        }
        let moved = |old: u32, new: u32| old.abs_diff(new) >= (old / 8).max(64);
        let changed = moved(self.sent_display.width, display.width)
            || moved(self.sent_display.height, display.height);
        if changed && self.resized_at.elapsed() >= RESIZE_PAUSE && session.info().accepted > 0 {
            session.send(ViewerFrame::SetMode(full_set_mode(None, display, prefs)));
            self.sent_display = display;
        }
    }

    fn send_keys(&self, events: Vec<InputEvent>) {
        if let (Some(session), true) = (&self.session, self.control_on) {
            if !session.input_ready() {
                return;
            }
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
    pub fn apply(&mut self, action: &ToolbarAction, common: &mut Common, display: FrameSize) {
        // The picture area, not the window with its toolbar (Gemini 3/18);
        // what goes out as `SET_MODE` is the size `follow_size` compares with.
        let display = self.measured.unwrap_or(display);
        // A new choice of „Codec“, „Dekodierer“, „Bibliothek“ or „Anzeige“:
        // the notice of the one before goes; one that still holds is named
        // again (Gemini 3/23: old errors stood over the working picture).
        let choice = matches!(
            action,
            ToolbarAction::Codec(_)
                | ToolbarAction::Decoder(_)
                | ToolbarAction::DecoderLibrary(_)
                | ToolbarAction::Display(_)
        );
        if let Some(session) = self.session.as_ref().filter(|_| choice) {
            session.info().notice = None;
            session.shared.direct.forget_noted();
        }
        match action {
            // The source shown already, or none the host could name (an empty
            // monitor list): the host keeps its source, so the view keeps its
            // picture too (Codex 3/15: it dropped it one-sidedly).
            ToolbarAction::Source(source) if self.same_source(source) => {}
            ToolbarAction::Source(source) => {
                self.source = source.clone();
                // A session opened again (a codec choice, Codex 6) starts from
                // the target: it names the chosen source, not the first one.
                self.target.source = source.clone();
                if let Some(session) = &self.session {
                    self.widget.source_changed(session);
                    let mode = full_set_mode(source.clone(), display, &common.prefs);
                    session.send(ViewerFrame::SetMode(mode));
                    self.sent_display = display;
                }
            }
            ToolbarAction::RefreshWindows => {
                if let Some(session) = &self.session {
                    session.send(ViewerFrame::ListWindows);
                }
            }
            ToolbarAction::Control(on) => self.take_control(common, display, *on),
            ToolbarAction::Codec(codec) => {
                if let Some(session) = &self.session {
                    // The viewer's own switch to the tiles ends with a new
                    // choice (user rule 2026-11-18).
                    session.codec_chosen(*codec);
                    let mode = full_set_mode(None, display, &common.prefs);
                    session.send(ViewerFrame::SetMode(mode));
                    self.sent_display = display;
                }
            }
            // P5 (plan 2026-11-17): „Monitor“ and „Auflösung“ go live to a
            // host that answered `live` (P9); an older one takes them with
            // the hello, so the session opens again.
            ToolbarAction::VirtualDisplay(choice) => {
                if super::screen_toolbar_ui::apply_virtual_display(*choice)
                    && !super::screen_toolbar_ui::send_live(self.session.as_ref(), *choice)
                {
                    self.reconnect(common, display);
                }
            }
            // „Höchste Datenrate“ at once; the relay drops it for a host
            // without `rate_ceiling_v1` and the statistics say so (BK6).
            ToolbarAction::MaxRate(rate) => {
                if let Some(session) = &self.session {
                    session.send(ViewerFrame::RateCeiling(*rate));
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
            ToolbarAction::Audio(on) => super::screen_audio::switch(self.session.as_ref(), *on),
            _ => {}
        }
    }

    /// Whether `source` is the one shown now; „Ganzer Bildschirm“ is the
    /// primary monitor (`whole_screen`), and no source at all changes nothing.
    fn same_source(&self, source: &Option<SourceId>) -> bool {
        let hello = self.session.as_ref().and_then(|s| s.info().hello.clone());
        let whole = hello
            .as_ref()
            .and_then(super::screen_toolbar_ui::whole_screen);
        let effective = |source: &Option<SourceId>| source.clone().or_else(|| whole.clone());
        effective(source).is_none() || effective(source) == effective(&self.source)
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
pub fn device_list(devices: &Devices) -> Vec<super::screen_devices::DeviceEntry> {
    let state = devices.lock();
    state.profiles.as_ref().map_or_else(Vec::new, |profiles| {
        entries(profiles, crate::share::core_now_secs())
    })
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[test]
    fn screen_metadata_runtime_task_resize_is_sent_only_once_the_size_rests() {
        let shared = super::super::screen_session::Shared::new(false, 0);
        shared.update(|info| info.accepted = 1);
        let session = ViewerSession::fixture(shared);
        let target = ScreenTarget::direct("resize-fixture", Some("resize-fixture"));
        let start = FrameSize::new(1000, 800);
        let mut view = ViewState::with_session(target, session, start, false);
        let prefs = ViewerPrefs::default();
        let long_ago = || Instant::now().checked_sub(RESIZE_PAUSE * 2).unwrap();
        // Still dragging: every new size restarts the pause.
        view.follow_size(FrameSize::new(1600, 1000), &prefs);
        view.resized_at = long_ago();
        view.follow_size(FrameSize::new(1700, 1050), &prefs);
        assert_eq!(view.sent_display, start);
        view.follow_size(FrameSize::new(1700, 1050), &prefs);
        assert_eq!(view.sent_display, start, "the pause has not passed");
        // The size rested for the pause: it goes out once.
        view.resized_at = long_ago();
        view.follow_size(FrameSize::new(1700, 1050), &prefs);
        assert_eq!(view.sent_display, FrameSize::new(1700, 1050));
        // A small change is never sent.
        view.follow_size(FrameSize::new(1710, 1060), &prefs);
        view.resized_at = long_ago();
        view.follow_size(FrameSize::new(1710, 1060), &prefs);
        assert_eq!(view.sent_display, FrameSize::new(1700, 1050));
    }
}
