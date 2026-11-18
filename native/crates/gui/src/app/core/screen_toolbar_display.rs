//! „Monitor“ and „Auflösung“ of the viewer's toolbar (plan 2026-11-17 P5;
//! user: „ich will per wahl auch auf den eigenen monitor wechseln können,
//! damit ich die auflösung einstellen kann“): „Monitor: Automatisch /
//! Physischer Monitor / Eigener (virtuell)“ and „Auflösung: Fenstergröße
//! folgen / a size of the list / Eigene Größe …“ (B×H in a field beside the
//! list, because a combo box of egui 0.29 closes on every click). Both are
//! stored at once (`screen_prefs::set_virtual_display_choice`, as `se screen
//! view --virtual-display --display-size`). A host that answered `live`
//! (P9, `set_display.rs`) takes a change within the session (`SET_DISPLAY`,
//! `send_live`): it switches its display and opens only capture and encoder
//! again; an older host takes the wish with the hello only, so the session
//! opens again for it. The state line names what
//! the host did (`ScreenHelloOk.virtual_display`) and, once the statistics
//! name another one, what it does now (`HostStats.chain.virtual_display`).
use eframe::egui::{self, RichText};

use super::super::screen_prefs::{VirtualDisplayChoice, VirtualDisplayWish};
use super::{ToolbarAction, ToolbarInput};
use crate::screen::core::virtual_display::{parse_size, size_text, SIZE_CHOICES};

/// The short text of the closed „Monitor“ list.
fn wish_text(wish: VirtualDisplayWish) -> &'static str {
    match wish {
        VirtualDisplayWish::Auto => "Automatisch",
        VirtualDisplayWish::Off => "physisch",
        VirtualDisplayWish::Request => "eigener (virtuell)",
    }
}

/// „Monitor“, „Auflösung“ and the state line.
pub(super) fn display_rows(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let current = super::super::screen_prefs::virtual_display_choice();
    let mut chosen = current;
    let _ = super::wrap::item(ui, "monitor", |ui| {
        ({
            let text = format!("Monitor: {}", wish_text(current.wish));
            super::wrap::fitted(
                ui,
                egui::ComboBox::from_id_salt("screen_virtual_display"),
                text,
            )
        })
        .show_ui(ui, |ui| {
            for wish in VirtualDisplayWish::ALL {
                ui.selectable_value(&mut chosen.wish, wish, wish.label());
            }
        })
        .response
        .on_hover_text(
            "Eigener Bildschirm am Gerät in der Größe dieses Fensters oder der gewählten \
             Auflösung. Automatisch: nur, wenn am Gerät kein Monitor hängt.",
        )
    });
    let custom_id = ui.make_persistent_id("screen_display_size_custom");
    let mut custom: Option<String> = ui
        .data(|data| data.get_temp::<Option<String>>(custom_id))
        .flatten();
    let shown = current.size.map_or("Fenstergröße".to_string(), size_text);
    let _ = super::wrap::item(ui, "size", |ui| {
        ({
            let text = format!("Auflösung: {shown}");
            super::wrap::fitted(
                ui,
                egui::ComboBox::from_id_salt("screen_display_size"),
                text,
            )
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut chosen.size, None, "Fenstergröße folgen");
            for size in SIZE_CHOICES {
                ui.selectable_value(&mut chosen.size, Some(size), size_text(size));
            }
            if ui.selectable_label(false, "Eigene Größe …").clicked() {
                let start = current
                    .size
                    .map(|size| format!("{}x{}", size.width, size.height));
                custom = Some(start.unwrap_or_default());
            }
        })
        .response
        .on_hover_text("Wirkt, wenn das Gerät einen eigenen Bildschirm zeigt.")
    });
    if let Some(text) = custom.as_mut() {
        let field = egui::TextEdit::singleline(text)
            .hint_text("2560x1440")
            .desired_width(90.0);
        let edited = ui.add(field);
        let parsed = parse_size(text);
        let entered = edited.lost_focus() && ui.input(|state| state.key_pressed(egui::Key::Enter));
        let applied = ui
            .add_enabled(parsed.is_some(), egui::Button::new("Übernehmen"))
            .clicked();
        if (applied || entered) && parsed.is_some() {
            chosen.size = parsed;
            custom = None;
        } else if ui.small_button("Abbrechen").clicked() {
            custom = None;
        }
    }
    ui.data_mut(|data| data.insert_temp(custom_id, custom));
    if chosen != current {
        out.push(ToolbarAction::VirtualDisplay(chosen));
    }
    if let Some(line) = state_line(input) {
        ui.label(RichText::new(line).small());
    }
}

/// What the host did with the wish, and what it does now when the
/// statistics name another state.
fn state_line(input: &ToolbarInput) -> Option<String> {
    state_text(input.info?, input.source)
}

/// The statistics' display key without its `;source=` part, and that source.
fn split_source(key: &str) -> (&str, Option<&str>) {
    match key.rsplit_once(";source=") {
        Some((key, source)) => (key, Some(source)),
        None => (key, None),
    }
}

/// `monitor:<id>` as the toolbar names it.
fn source_text(source: &str) -> String {
    match source.split_once(':') {
        Some(("monitor", id)) => format!("Monitor {id}"),
        Some(("window", id)) => format!("Fenster {id}"),
        _ => source.to_string(),
    }
}

/// The state line of a session (user rule 2026-11-18): a host that sent no
/// report (older than P5) did not take „Monitor“/„Auflösung“; a session the
/// host moved to another monitor (`source`, `source_chosen` of the report,
/// `;source=` of the statistics) names the source it really shows:
/// „Quelle: <gewählt> → jetzt <tatsächlich>“. `chosen` is the source of the
/// toolbar's list.
pub(super) fn state_text(
    info: &super::super::screen_session::SessionInfo,
    chosen: Option<&crate::screen::core::window_list::SourceId>,
) -> Option<String> {
    let hello = info.hello.as_ref()?;
    let Some(report) = hello.virtual_display.as_ref() else {
        return Some("Monitor und Auflösung: vom Gerät nicht unterstützt".into());
    };
    // The report's own text without its source part: the line names it once.
    let mut plain = report.clone();
    plain.source = None;
    plain.source_chosen = None;
    let now = info
        .stats
        .as_ref()
        .and_then(|stats| stats.chain.as_ref())
        .and_then(|chain| chain.virtual_display.as_deref());
    let (now_key, now_source) = now.map_or((None, None), |key| {
        let (key, source) = split_source(key);
        (Some(key), source)
    });
    let mut line = plain.text();
    if let Some(key) = now_key.filter(|key| *key != plain.chain_key()) {
        line.push_str(&format!(" · jetzt {key}"));
    }
    let shown = now_source.or(report.source.as_deref());
    if let Some(shown) = shown {
        let wanted = report
            .source_chosen
            .clone()
            .or_else(|| chosen.map(ToString::to_string));
        match wanted.filter(|wanted| wanted != shown) {
            Some(wanted) => line.push_str(&format!(
                " · Quelle: {} → jetzt {}",
                source_text(&wanted),
                source_text(shown)
            )),
            None => line.push_str(&format!(" · Quelle: {}", source_text(shown))),
        }
    }
    Some(line)
}

/// P9: sends a new „Monitor“ or „Auflösung“ as `SET_DISPLAY` when the host
/// answered `live` for this session; `false` when the session must open
/// again with it.
pub fn send_live(
    session: Option<&super::super::screen_session::ViewerSession>,
    choice: VirtualDisplayChoice,
) -> bool {
    let Some(session) = session else {
        return false;
    };
    if !crate::screen::core::set_display::negotiated(session.info().hello.as_ref()) {
        return false;
    }
    session.send(crate::screen::core::wire_control::ViewerFrame::SetDisplay(
        choice,
    ));
    true
}

/// Stores a new „Monitor“ or „Auflösung“ (`ToolbarAction::VirtualDisplay`);
/// `true` when the session must open again with it.
pub fn apply_virtual_display(choice: VirtualDisplayChoice) -> bool {
    let before = super::super::screen_prefs::virtual_display_choice();
    if before == choice {
        return false;
    }
    super::super::screen_prefs::set_virtual_display_choice(Some(choice.wish), Some(choice.size))
        .is_ok()
}

#[cfg(test)]
#[path = "wahlgilt_display_task_tests.rs"]
mod wahlgilt_display_task_tests;
