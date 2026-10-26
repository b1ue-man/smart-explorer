//! Area „Bildschirmfreigabe (Wayland)“ of the tab „Bildschirm“ (SCR-M5,
//! Linux): whether the system dialog's answer is stored, „Freigabe
//! vergessen“, and on wlroots desktops the hint that a lasting share needs
//! `XDPW_PERSIST_MODE`. The area exists only in a Wayland session. The
//! stored grant is read from the token file about every two seconds while
//! the tab is drawn; „Freigabe vergessen“ deletes it (sessions that run
//! go on, the next one asks again).
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use crate::app::theme;
use crate::screen::os::portal_tokens::{self, TokenKind};
use crate::screen::os::session_detect::{self, SessionKind};

/// How often the token file is read while the tab is drawn.
const REREAD: Duration = Duration::from_secs(2);
/// How long the notice after „Freigabe vergessen“ stays.
const NOTICE_FOR: Duration = Duration::from_secs(6);
/// The hint for wlroots desktops (U19-8).
pub(crate) const WLROOTS_HINT: &str =
    "Dauerhafte Freigabe verlangt XDPW_PERSIST_MODE=permanent in der Umgebung des Portal-Dienstes.";

#[derive(Default)]
struct PortalUi {
    /// `Some(saved_at)` when a grant is stored.
    saved_at: Option<Option<i64>>,
    read_at: Option<Instant>,
    notice: Option<(String, Instant)>,
}

fn state() -> MutexGuard<'static, PortalUi> {
    static STATE: OnceLock<Mutex<PortalUi>> = OnceLock::new();
    STATE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// „vorhanden seit {Datum}“ for a grant saved at `saved_at` (Unix seconds).
pub(crate) fn since_text(saved_at: i64) -> String {
    match chrono::DateTime::from_timestamp(saved_at, 0) {
        Some(time) => format!(
            "vorhanden seit {}",
            time.with_timezone(&chrono::Local).format("%d.%m.%Y")
        ),
        None => "vorhanden".into(),
    }
}

/// Draws the area; nothing outside a Wayland session.
pub(crate) fn ui(ui: &mut egui::Ui) {
    if session_detect::detect_process() != SessionKind::Wayland {
        return;
    }
    let (stored, notice) = {
        let mut current = state();
        let stale = current.read_at.is_none_or(|at| at.elapsed() >= REREAD);
        if stale {
            current.saved_at = Some(portal_tokens::saved_at(TokenKind::ScreenCast));
            current.read_at = Some(Instant::now());
        }
        if current
            .notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= NOTICE_FOR)
        {
            current.notice = None;
        }
        (
            current.saved_at.flatten(),
            current.notice.as_ref().map(|(text, _)| text.clone()),
        )
    };
    theme::section(ui, "Bildschirmfreigabe (Wayland)");
    let line = match stored {
        Some(saved_at) => since_text(saved_at),
        None => "keine".into(),
    };
    ui.label(format!("Gespeicherte Freigabe: {line}"));
    ui.horizontal(|ui| {
        let forget = ui.add_enabled(stored.is_some(), egui::Button::new("Freigabe vergessen"));
        if forget.clicked() {
            let result = portal_tokens::forget();
            let mut current = state();
            current.saved_at = None;
            current.read_at = None;
            let text = match result {
                Ok(_) => "Gespeicherte Bildschirmfreigabe vergessen.".to_string(),
                Err(error) => format!("Bildschirmfreigabe nicht vergessen: {error}"),
            };
            current.notice = Some((text, Instant::now()));
            ui.ctx().request_repaint();
        }
        if let Some(text) = &notice {
            ui.label(RichText::new(text).small().color(theme::muted(ui)));
        }
    });
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if session_detect::is_wlroots_desktop(&desktop) {
        ui.small("Die dauerhafte Bildschirmfreigabe wird vom Portal-Dienst dieses Desktops gesteuert.");
        egui::CollapsingHeader::new("Einrichtungsdiagnose").id_salt("wayland_portal_diagnostics").show(ui, |ui| { ui.label(RichText::new(WLROOTS_HINT).small().color(theme::muted(ui))); });
    }
    // The notice and the file are checked again soon.
    ui.ctx().request_repaint_after(REREAD);
}
