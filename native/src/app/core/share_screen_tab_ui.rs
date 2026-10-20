//! Tab „Bildschirm“ (tab 9) of „Geräte & Freigaben“ (SCR-M2): the screen
//! sessions this device hosts with „Beenden“ and „Alle beenden“, and the
//! open confirmations for the screen, answered like in the consent window
//! („Diesmal erlauben“, „Immer erlauben“, „Ablehnen“). A reader thread takes
//! both from the running background worker once a second while the tab is
//! drawn; it never starts a worker (without one there are no sessions).
//! Who may watch or control this device is set per device on „Zugriff“.
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use crate::app::theme;
use crate::screen::core::registry::SessionView;
use crate::share::access_approval::{ApprovalDecision, ApprovalView};
use crate::share::access_grant::Capability;

/// Interval of the reader.
const POLL: Duration = Duration::from_secs(1);
/// The reader ends this long after the tab was last drawn.
const IDLE: Duration = Duration::from_secs(10);

#[derive(Default)]
struct TabState {
    sessions: Vec<SessionView>,
    approvals: Vec<ApprovalView>,
    loaded: bool,
    error: Option<String>,
    reader: bool,
    drawn: Option<Instant>,
}

fn state() -> &'static Arc<Mutex<TabState>> {
    static STATE: OnceLock<Arc<Mutex<TabState>>> = OnceLock::new();
    STATE.get_or_init(Arc::default)
}

fn lock(state: &Mutex<TabState>) -> MutexGuard<'_, TabState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether an open confirmation is about the screen.
pub(crate) fn is_screen_request(view: &ApprovalView) -> bool {
    matches!(
        view.capability,
        Capability::ScreenView | Capability::ScreenControl
    )
}

/// One row of a running session.
pub(crate) fn session_row(session: &SessionView) -> String {
    let mode = match session.mode {
        crate::screen::core::wire_hello::SessionMode::Control => "steuert",
        crate::screen::core::wire_hello::SessionMode::Preview => "Vorschau",
        _ => "sieht zu",
    };
    let codec = session.codec.as_deref().unwrap_or("–");
    format!(
        "{} · {mode} · {} · {} · {codec} · {} · {} Bilder/s · {} kbit/s",
        session.device_name,
        session.source,
        session.state.key(),
        session.path,
        session.fps,
        session.bitrate_kbps
    )
}

/// Draws the tab (called from `share_window_ui.rs` for `TAB_SCREEN`).
pub(crate) fn ui(ui: &mut egui::Ui) {
    let shared = state().clone();
    {
        let mut current = lock(&shared);
        current.drawn = Some(Instant::now());
        if !current.reader {
            current.reader = true;
            let (slot, ctx) = (shared.clone(), ui.ctx().clone());
            let started = std::thread::Builder::new()
                .name("share-screen-tab".into())
                .spawn(move || read(&slot, &ctx));
            if started.is_err() {
                current.reader = false;
            }
        }
    }
    let (sessions, approvals, loaded, error) = {
        let current = lock(&shared);
        (
            current.sessions.clone(),
            current.approvals.clone(),
            current.loaded,
            current.error.clone(),
        )
    };
    ui.label(
        RichText::new(
            "Wer diesen Bildschirm ansehen oder steuern darf, stellst du je Gerät unter \
             „Zugriff“ ein (Rechte „Bildschirm ansehen“ und „Bildschirm steuern“).",
        )
        .small()
        .color(theme::muted(ui)),
    );
    open_all_devices(ui);
    if let Some(error) = &error {
        ui.colored_label(theme::danger(ui), format!("Nicht lesbar: {error}"));
    }
    theme::section(ui, "Offene Rückfragen");
    if approvals.is_empty() {
        ui.label("Keine offene Rückfrage für den Bildschirm.");
    }
    for request in &approvals {
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::share::consent_launch::request_line(request));
            if ui.button("Diesmal erlauben").clicked() {
                decide(request, ApprovalDecision::Once);
            }
            if ui.button("Immer erlauben").clicked() {
                decide(request, ApprovalDecision::Always);
            }
            if ui.button("Ablehnen").clicked() {
                decide(request, ApprovalDecision::Deny);
            }
        });
    }
    crate::app::share_screen_codec_ui::ui(ui);
    #[cfg(target_os = "linux")]
    crate::app::share_screen_portal_ui::ui(ui);
    theme::section(ui, "Laufende Sitzungen");
    if !loaded {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Lese die Sitzungen …");
        });
        return;
    }
    if sessions.is_empty() {
        ui.label("Kein Gerät sieht diesen Bildschirm.");
        return;
    }
    for session in &sessions {
        ui.horizontal_wrapped(|ui| {
            ui.label(session_row(session));
            if ui.button("Beenden").clicked() {
                stop(Some(session.id.clone()));
            }
        });
    }
    if sessions.len() > 1 && ui.button("Alle beenden").clicked() {
        stop(None);
    }
}

/// „Alle Geräte“ (SCR-M7): the group view of every device in its own window.
fn open_all_devices(ui: &mut egui::Ui) {
    let id = egui::Id::new("screen_tab_launch_error");
    let button = crate::app::screen_launch::all_devices_button();
    if crate::app::screen_launch::show_button(ui, &button) {
        let result = crate::app::screen_launch::open_grid(None);
        ui.data_mut(|data| match result {
            Ok(()) => data.remove::<String>(id),
            Err(error) => data.insert_temp(id, error),
        });
    }
    if let Some(error) = ui.data(|data| data.get_temp::<String>(id)) {
        ui.colored_label(theme::danger(ui), error);
    }
}

fn stop(session: Option<String>) {
    let _ = std::thread::Builder::new()
        .name("share-screen-stop".into())
        .spawn(move || {
            let _ = crate::daemon::screen_stop(session);
        });
}

fn decide(request: &ApprovalView, decision: ApprovalDecision) {
    let id = request.id.as_str().to_string();
    let _ = std::thread::Builder::new()
        .name("share-screen-decide".into())
        .spawn(move || {
            let _ = crate::daemon::decide_access_approval(&id, decision);
        });
}

/// Reads sessions and confirmations once a second until the tab rests.
fn read(shared: &Arc<Mutex<TabState>>, ctx: &egui::Context) {
    loop {
        let idle = lock(shared)
            .drawn
            .is_none_or(|drawn| drawn.elapsed() > IDLE);
        if idle {
            lock(shared).reader = false;
            return;
        }
        let sessions = crate::daemon::screen_sessions();
        let approvals = crate::daemon::access_approvals();
        {
            let mut current = lock(shared);
            current.error = sessions
                .as_ref()
                .err()
                .or(approvals.as_ref().err())
                .cloned();
            if let Ok((_, sessions)) = sessions {
                current.sessions = sessions;
                current.loaded = true;
            }
            if let Ok(pending) = approvals {
                current.approvals = pending.into_iter().filter(is_screen_request).collect();
            }
        }
        ctx.request_repaint();
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::session_state::SessionState;
    use crate::screen::core::wire_hello::SessionMode;

    #[test]
    fn screen_task_gui_tab_rows() {
        let session = SessionView {
            id: "4".into(),
            device_id: "laptop".into(),
            device_name: "Laptop".into(),
            relation: "direct".into(),
            room_id: None,
            mode: SessionMode::Control,
            source: "synthetic".into(),
            codec: Some("tiles".into()),
            encoder: Some("tiles".into()),
            path: "relay".into(),
            started_at: 0,
            bitrate_kbps: 1200,
            fps: 30,
            controlling: true,
            state: SessionState::Streaming,
        };
        assert_eq!(
            session_row(&session),
            "Laptop · steuert · synthetic · streaming · tiles · relay · 30 Bilder/s · 1200 kbit/s"
        );
        let request: ApprovalView = serde_json::from_value(serde_json::json!({
            "id": "ab".repeat(16),
            "device_name": "Laptop",
            "device_id": "laptop",
            "fingerprint": "fp",
            "relation": "direct",
            "room": null,
            "capability": "screen_view",
            "account_label": null,
            "command": null,
            "requested_at": 0,
            "surface": "window",
        }))
        .unwrap();
        assert!(is_screen_request(&request));
        let exec = ApprovalView {
            capability: Capability::ExecCommands,
            ..request
        };
        assert!(!is_screen_request(&exec));
    }
}
