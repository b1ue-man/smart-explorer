//! The indicator window `smart_explorer --screen-indicator` (SCR-M2, C7):
//! the background worker starts it while another device watches or controls
//! this screen. It stays on top, names every device that watches („…
//! sieht deinen Bildschirm“, „… steuert“) with what it sees, and ends one
//! session („Beenden“) or all („Alle beenden“). Closing the window ends
//! every session it shows: no session runs without the indicator. It reads
//! the sessions from the worker as they change and closes itself shortly
//! after the last one ended. A new update-stop request closes it without
//! ending sessions; the worker ends them on its own (V8).
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use crate::app::theme;
use crate::screen::core::registry::SessionView;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_hello::SessionMode;

/// The window closes this long after the last session ended.
const EMPTY_LINGER: Duration = Duration::from_secs(3);
/// Pause after a failed read of the sessions.
const RETRY: Duration = Duration::from_secs(1);

/// Runs the indicator until no session is left or the user closes it.
pub(crate) fn run_screen_indicator() -> eframe::Result<()> {
    crate::app::update_stop_ui::arm_window();
    // The indicator never takes the keyboard: keys of a controlling viewer
    // belong to the program it controls (Windows: shown without activation;
    // X11 login greeter: a NOTIFICATION, which greeters do not focus).
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Bildschirmfreigabe – Smart Explorer")
        .with_inner_size([440.0, 220.0])
        .with_min_inner_size([340.0, 150.0])
        .with_always_on_top()
        .with_active(false)
        .with_icon(crate::window_icon());
    if let Some(window_type) = crate::app::platform_helpers::indicator_window_type() {
        viewport = viewport.with_window_type(window_type);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Smart Explorer Bildschirmfreigabe",
        options,
        Box::new(|cc| Ok(Box::new(IndicatorWindow::new(&cc.egui_ctx)))),
    )
}

/// Whether a session in `state` shows this screen to its viewer now.
pub(crate) fn watching(state: SessionState) -> bool {
    matches!(state, SessionState::Starting | SessionState::Streaming) || state.is_paused()
}

/// „Laptop sieht deinen Bildschirm (Raum Ops)“ and what is shown.
pub(crate) fn session_line(session: &SessionView) -> String {
    let verb = match session.mode {
        SessionMode::Control => "steuert deinen Bildschirm",
        SessionMode::Preview => "sieht eine Vorschau deines Bildschirms",
        SessionMode::View | SessionMode::Probe => "sieht deinen Bildschirm",
    };
    let shown = match session.source.split_once(':') {
        Some(("monitor", id)) => format!("Bildschirm {id}"),
        Some(("window", _)) => "ein Fenster".to_string(),
        _ => "das Testbild".to_string(),
    };
    let room = if session.room_id.is_some() {
        " (Raum)"
    } else {
        ""
    };
    format!("{}{room} {verb} – {shown}", session.device_name)
}

#[derive(Clone, Default)]
struct State {
    sessions: Vec<SessionView>,
    loaded: bool,
    error: Option<String>,
}

struct IndicatorWindow {
    state: Arc<Mutex<State>>,
    empty_since: Option<Instant>,
    /// The window closes itself (no session left, update stop): no session
    /// is ended for it.
    self_close: bool,
}

impl IndicatorWindow {
    fn new(ctx: &egui::Context) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let (slot, poll_ctx) = (Arc::downgrade(&state), ctx.clone());
        let _ = std::thread::Builder::new()
            .name("screen-indicator-watch".into())
            .spawn(move || watch(&slot, &poll_ctx));
        Self {
            state,
            empty_since: None,
            self_close: false,
        }
    }

    fn snapshot(&self) -> State {
        lock(&self.state).clone()
    }

    fn close_self(&mut self, ctx: &egui::Context) {
        self.self_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn body(ui: &mut egui::Ui, state: &State, watched: &[&SessionView]) {
        if let Some(error) = &state.error {
            let text = format!("Sitzungen nicht lesbar: {error}");
            ui.colored_label(theme::danger(ui), text);
        }
        if watched.is_empty() {
            let text = if state.loaded {
                "Kein Gerät sieht diesen Bildschirm mehr."
            } else {
                "Lese die Bildschirmsitzungen …"
            };
            ui.label(text);
            return;
        }
        ui.heading("Dein Bildschirm wird angesehen");
        ui.add_space(4.0);
        for session in watched {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(session_line(session)).strong());
                if ui.button("Beenden").clicked() {
                    stop(Some(session.id.clone()));
                }
            });
        }
        if watched.len() > 1 {
            ui.add_space(4.0);
            if ui.button("Alle beenden").clicked() {
                stop(None);
            }
        }
        ui.add_space(6.0);
        let hint = "Schließen dieses Fensters beendet alle Sitzungen. Wer zusehen darf, \
                    stellst du unter „Geräte & Freigaben“ → „Zugriff“ ein.";
        ui.label(RichText::new(hint).small().color(theme::muted(ui)));
    }
}

impl eframe::App for IndicatorWindow {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // V8: a new update-stop request closes the window; the worker ends
        // the sessions itself.
        if crate::app::update_stop_ui::close_requested(ctx) {
            self.close_self(ctx);
        }
        let state = self.snapshot();
        let watched: Vec<&SessionView> = state
            .sessions
            .iter()
            .filter(|session| watching(session.state))
            .collect();
        if state.loaded && watched.is_empty() {
            let since = *self.empty_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= EMPTY_LINGER {
                self.close_self(ctx);
            }
        } else {
            self.empty_since = None;
        }
        ctx.request_repaint_after(Duration::from_millis(500));
        egui::CentralPanel::default().show(ctx, |ui| Self::body(ui, &state, &watched));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Closed by the user: every session it showed ends (C7).
        if !self.self_close {
            let _ = crate::daemon::screen_stop(None);
        }
    }
}

/// Ends one session, or all, on a thread of its own.
fn stop(session: Option<String>) {
    let _ = std::thread::Builder::new()
        .name("screen-indicator-stop".into())
        .spawn(move || {
            let _ = crate::daemon::screen_stop(session);
        });
}

/// Follows the sessions of the worker until the window goes. After a failed
/// read the next one asks for the whole list again: without a worker there
/// is no session, so the window closes.
fn watch(slot: &Weak<Mutex<State>>, ctx: &egui::Context) {
    let mut revision = None;
    loop {
        let read = match revision {
            None => crate::daemon::screen_sessions(),
            Some(since) => crate::daemon::screen_watch(since),
        };
        let Some(state) = slot.upgrade() else {
            return;
        };
        let failed = {
            let mut current = lock(&state);
            match read {
                Ok((next, sessions)) => {
                    revision = Some(next);
                    current.sessions = sessions;
                    current.loaded = true;
                    current.error = None;
                    false
                }
                Err(error) => {
                    revision = None;
                    current.error = Some(error);
                    true
                }
            }
        };
        drop(state);
        ctx.request_repaint();
        if failed {
            std::thread::sleep(RETRY);
        }
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(mode: SessionMode, source: &str, state: SessionState, room: bool) -> SessionView {
        SessionView {
            id: "1".into(),
            device_id: "laptop".into(),
            device_name: "Laptop".into(),
            relation: if room { "room" } else { "direct" }.into(),
            room_id: room.then(|| "room-1".into()),
            mode,
            source: source.into(),
            codec: None,
            encoder: None,
            path: "direct".into(),
            started_at: 0,
            bitrate_kbps: 0,
            fps: 0,
            controlling: false,
            state,
        }
    }

    #[test]
    fn screen_task_gui_indicator_lines() {
        let watching_view = view(
            SessionMode::View,
            "monitor:1",
            SessionState::Streaming,
            false,
        );
        assert_eq!(
            session_line(&watching_view),
            "Laptop sieht deinen Bildschirm – Bildschirm 1"
        );
        let control = view(
            SessionMode::Control,
            "window:0x2a",
            SessionState::Starting,
            true,
        );
        assert_eq!(
            session_line(&control),
            "Laptop (Raum) steuert deinen Bildschirm – ein Fenster"
        );
        let test = view(
            SessionMode::View,
            "synthetic",
            SessionState::Streaming,
            false,
        );
        assert!(session_line(&test).ends_with("das Testbild"));
        // A session that waits for the host user does not watch yet.
        assert!(!watching(SessionState::WaitingForApproval));
        assert!(watching(SessionState::Streaming));
        assert!(watching(SessionState::SecureDesktop));
        assert!(watching(SessionState::ViewerHidden));
        assert!(!watching(SessionState::Ended));
    }
}
