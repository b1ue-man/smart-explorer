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
    let _ = egui::ComboBox::from_id_salt("screen_virtual_display")
        .selected_text(format!("Monitor: {}", wish_text(current.wish)))
        .show_ui(ui, |ui| {
            for wish in VirtualDisplayWish::ALL {
                ui.selectable_value(&mut chosen.wish, wish, wish.label());
            }
        })
        .response
        .on_hover_text(
            "Eigener Bildschirm am Gerät in der Größe dieses Fensters oder der gewählten \
             Auflösung. Automatisch: nur, wenn am Gerät kein Monitor hängt.",
        );
    let custom_id = ui.make_persistent_id("screen_display_size_custom");
    let mut custom: Option<String> = ui
        .data(|data| data.get_temp::<Option<String>>(custom_id))
        .flatten();
    let shown = current.size.map_or("Fenstergröße".to_string(), size_text);
    let _ = egui::ComboBox::from_id_salt("screen_display_size")
        .selected_text(format!("Auflösung: {shown}"))
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
        .on_hover_text("Wirkt, wenn das Gerät einen eigenen Bildschirm zeigt.");
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
    let info = input.info?;
    let report = info.hello.as_ref()?.virtual_display.as_ref()?;
    let now = info
        .stats
        .as_ref()
        .and_then(|stats| stats.chain.as_ref())
        .and_then(|chain| chain.virtual_display.clone())
        .filter(|key| *key != report.chain_key());
    Some(match now {
        Some(key) => format!("{} · jetzt {key}", report.text()),
        None => report.text(),
    })
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
