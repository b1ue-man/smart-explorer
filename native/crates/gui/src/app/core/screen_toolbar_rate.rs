//! „Übertragung“ and „Datenrate“ of the viewer's toolbar (user 2026-11-25,
//! `screen_delivery.rs`; „Datenrate“ was „Höchste Datenrate“ of plan
//! 2026-11-14 „Bildkette“ B-09 and moved here from `screen_toolbar_codec.rs`).
//!
//! „Übertragung: UDP / TCP“, each entry with
//! its mode's hint as hover text; a host before „Übertragung“ greys both
//! entries out with the reason. „Datenrate: Automatisch (gemessen: X
//! Mbit/s) / Fester Wert“: under „Automatisch“ the host sends with the rate
//! it measured on the line, with „Fester Wert“ exactly with the value of the
//! field in Mbit/s. The value is stored at once
//! (`screen_prefs::set_max_rate_bps`) and goes out as `RATE_CEILING`, whose
//! wire meaning stays. „Neu messen“ lets the host measure the line again.
use std::time::Instant;

use eframe::egui::{self, RichText, WidgetText};

use super::super::screen_delivery::{self as delivery, Delivery, Measured};
use super::codec_rows::note_save;
use super::{wrap, ToolbarAction, ToolbarInput};

/// When „Übertragung“ was last chosen in this window (egui's memory).
fn delivery_changed_id() -> egui::Id {
    egui::Id::new("screen_delivery_changed")
}

/// When „Neu messen“ was last pressed in this window (egui's memory).
fn remeasure_id() -> egui::Id {
    egui::Id::new("screen_remeasure_pressed")
}

/// „Übertragung“ as a drop-down list; the closed list names what the host
/// runs when it runs otherwise (warning colour) or has not answered yet.
pub(super) fn delivery_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let chosen = delivery::delivery();
    let running = input.info.and_then(delivery::running);
    let unknown = input.info.is_some_and(delivery::unknown_to_host);
    let changed_at = ui.data(|data| data.get_temp::<Instant>(delivery_changed_id()));
    let window = super::applied::ANSWER_WINDOW;
    let pending = changed_at.is_some_and(|at| at.elapsed() < window);
    let text = RichText::new(delivery::list_text(chosen, running, pending, unknown));
    let other = unknown || (running.is_some_and(|running| running != chosen) && !pending);
    let text: WidgetText = if other {
        text.color(crate::app::theme::warning(ui)).into()
    } else if pending && running.is_some_and(|running| running != chosen) {
        text.italics().into()
    } else {
        text.into()
    };
    let mut picked = chosen;
    let _ = wrap::item(ui, "delivery", |ui| {
        let list = egui::ComboBox::from_id_salt("screen_delivery");
        wrap::fitted(ui, list, text)
            .show_ui(ui, |ui| {
                wrap::popup(ui, Delivery::ALL.iter().map(|mode| mode.label()));
                for mode in Delivery::ALL {
                    let entry = egui::SelectableLabel::new(picked == mode, mode.label());
                    if unknown {
                        let reason = format!("{}: {}", mode.label(), delivery::UNKNOWN_REASON);
                        let _ = ui.add_enabled(false, entry).on_disabled_hover_text(reason);
                    } else if ui.add(entry).on_hover_text(mode.hint()).clicked() {
                        picked = mode;
                    }
                }
            })
            .response
            .on_hover_text(chosen.hint())
    });
    if picked != chosen {
        ui.data_mut(|data| data.insert_temp(delivery_changed_id(), Instant::now()));
        out.push(ToolbarAction::Delivery(picked));
    }
}

/// The width of the „Datenrate“ field in points: a typed value scrolls
/// inside it (17a: egui's `DragValue` grew with the typed text, past the
/// toolbar's edge).
const RATE_FIELD_WIDTH: f32 = 70.0;
const RATE_UNIT: &str = "Mbit/s";
/// The entry of „Datenrate“ with a value of the user's own.
const FIXED: &str = "Fester Wert";
const RATE_HINT: &str = "Automatisch: das Gerät misst die Leitung und sendet mit der \
     gemessenen Rate, ohne sie zu drosseln. Fester Wert: das Gerät sendet genau mit diesem \
     Wert. Wirkt sofort, wenn das Gerät es unterstützt.";

/// Where the field's error waits for the next try (`rate_field`).
fn rate_error_id() -> egui::Id {
    egui::Id::new("screen_max_rate_error")
}

/// The field of „Datenrate“: the stored value while it is not typed into,
/// the typed text while it is; `Some` with the value in Mbit/s once a
/// parsable text is left (Enter or focus lost; a German comma is the
/// decimal point). Every key stroke went out as `RATE_CEILING` before
/// (Gemini 10).
fn rate_field(ui: &mut egui::Ui, mbit: f64) -> Option<f64> {
    use crate::screen::core::rate_control::MIN_BITRATE_BPS;
    let id = egui::Id::new("screen_max_rate_text");
    let error_id = rate_error_id();
    // To the bit (Codex 4/20: two decimals turned 1,236 into 1,24 on a mere
    // click in and out), without trailing zeros, with the decimal comma.
    let shown = format!("{mbit:.6}");
    let shown = shown
        .trim_end_matches('0')
        .trim_end_matches('.')
        .replace('.', ",");
    let mut text = ui
        .data(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| shown.clone());
    // An id of its own (Codex 7/15): the automatic one counts the widgets
    // before it, so a row that appeared mid-edit („Gesteuert von …“) took
    // the focus away and the typed text with it.
    let field = egui::TextEdit::singleline(&mut text)
        .id(egui::Id::new("screen_max_rate_field"))
        // Never wider than the rest of its row (Gemini 8/11: a bar
        // narrower than the field ran it past the edge).
        .desired_width(wrap::text_edit_room(ui, RATE_FIELD_WIDTH))
        .clip_text(true);
    let edited = ui.add(field);
    wrap::note_editing(ui, &edited);
    let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
    if edited.gained_focus() || escaped {
        // A new try, or one given up (H6: the old error stood on).
        ui.data_mut(|data| data.remove::<String>(error_id));
    }
    if edited.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, text));
        return None;
    }
    ui.data_mut(|data| data.remove::<String>(id));
    // Escape leaves the field without its typed text (Gemini 3/8); a text
    // nobody changed is no new choice.
    if !edited.lost_focus() || escaped || text.trim() == shown {
        return None;
    }
    let minimum = MIN_BITRATE_BPS as f64 / 1e6;
    // The largest value a `u64` of bit/s holds (Codex 7/17: 1e30 went out
    // as `u64::MAX` without a word).
    let maximum = u64::MAX / 1_000_000;
    let parsed = text.trim().replace(',', ".").parse::<f64>().ok();
    let error = match parsed.filter(|value| value.is_finite()) {
        None => Some(format!("„{}“ ist keine Zahl", text.trim())),
        // Never raised in silence to the minimum (Codex 4/20).
        Some(value) if value < minimum => Some(format!(
            "mindestens {} Mbit/s",
            format!("{minimum}").replace('.', ",")
        )),
        Some(value) if value > maximum as f64 => Some(format!("höchstens {maximum} Mbit/s")),
        Some(_) => None,
    };
    ui.data_mut(|data| match &error {
        Some(error) => data.insert_temp(error_id, error.clone()),
        None => data.remove::<String>(error_id),
    });
    parsed.filter(|_| error.is_none())
}

/// The value the host's statistics name as the one it applies
/// (`STATS` → `ChainReport::rate_ceiling_kbps`, once a second).
fn reported(input: &ToolbarInput) -> Option<u64> {
    input
        .info
        .and_then(|info| info.stats.as_ref())
        .and_then(|stats| stats.chain.as_ref())
        .and_then(|chain| chain.rate_ceiling_kbps)
}

/// Whether the host reports `stored` as the value it applies. The report
/// counts whole kbit/s, so it proves only a value of whole kbit/s (Codex
/// 5/23: a report of the value before, rounded alike, confirmed a new one);
/// finer values are not called „übernommen“.
fn confirmed(input: &ToolbarInput, stored: Option<u64>) -> bool {
    stored.is_some_and(|bps| bps % 1000 == 0 && reported(input) == Some(bps / 1000))
}

/// Why a sent value `stored` is not called „übernommen“ (`confirmed`).
fn unconfirmed(input: &ToolbarInput, stored: Option<u64>) -> Option<&'static str> {
    let bps = stored?;
    Some(match reported(input) {
        None => "gesendet; noch keine Bestätigung des Geräts",
        Some(_) if bps % 1000 != 0 => "gesendet; das Gerät bestätigt nur ganze kbit/s",
        Some(_) => "gesendet; das Gerät meldet noch einen anderen Wert",
    })
}

/// „Datenrate“: „Automatisch“ with what the host measured, or „Fester
/// Wert“ with its field; a value chosen here goes out at once. A new value
/// starts at the measured rate, else at the session's current target.
pub(super) fn rate_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    use crate::screen::core::rate_control::MIN_BITRATE_BPS;
    let stored = super::super::screen_prefs::max_rate_bps();
    let negotiated = input.info.is_some_and(delivery::negotiated);
    let measured = input.info.and_then(delivery::measured_in);
    let automatic = delivery::automatic_text(measured, negotiated);
    let target = input
        .info
        .and_then(|info| info.stats.as_ref())
        .map(|stats| stats.target_kbps * 1000);
    let start = stored
        .or_else(|| measured.map(|measured| measured.kbps * 1000))
        .or(target);
    let mut mbit = start.unwrap_or(MIN_BITRATE_BPS).max(MIN_BITRATE_BPS) as f64 / 1e6;
    // The field of a value chosen in this pass follows in the next one,
    // measured with it (Codex 5/21: a row broke for a field not drawn).
    let shown = stored.is_some();
    let mut fixed = shown;
    let closed = if shown {
        format!("Datenrate: {FIXED}")
    } else {
        format!("Datenrate: {automatic}")
    };
    let _ = wrap::item(ui, "rate", |ui| {
        let list = egui::ComboBox::from_id_salt("screen_rate_mode");
        wrap::fitted(ui, list, closed)
            .show_ui(ui, |ui| {
                wrap::popup(ui, [automatic.as_str(), FIXED]);
                ui.selectable_value(&mut fixed, false, automatic.as_str());
                ui.selectable_value(&mut fixed, true, FIXED);
            })
            .response
            .on_hover_text(RATE_HINT)
    });
    let mut changed = fixed != shown;
    if !fixed {
        // „Automatisch“: no field, no error of it (H6).
        ui.data_mut(|data| data.remove::<String>(rate_error_id()));
    }
    if fixed && shown {
        // Field and unit break the row themselves when they do not fit the
        // rest of it (Gemini 5/8).
        wrap::reserve(ui, wrap::text_edit_width(RATE_FIELD_WIDTH));
        if let Some(typed) = rate_field(ui, mbit) {
            let typed = typed.max(MIN_BITRATE_BPS as f64 / 1e6);
            changed |= typed != mbit;
            mbit = typed;
        }
        wrap::reserve(ui, wrap::label_width(ui, RATE_UNIT));
        ui.label(RATE_UNIT);
        // After the unit, an entry of its own (Codex 6/21: between field
        // and unit it tore the unit away from its number).
        let error = ui.data(|data| data.get_temp::<String>(rate_error_id()));
        if let Some(error) = error.as_deref() {
            let warning = crate::app::theme::warning(ui);
            wrap::note(ui, RichText::new(error).color(warning));
        }
        // What the line can carry, beside the value of the user's own.
        if let Some(measured) = measured {
            let text = format!("gemessen: {} Mbit/s", delivery::mbit_text(measured.kbps));
            wrap::note(ui, RichText::new(text).small());
        }
    }
    // P9: what the session does with the value. A host that cannot keep it
    // ends the session with its reason (Codex 10/7, round 11 N1).
    if fixed && confirmed(input, stored) {
        // The host's own statistics name this value (Codex 3/22): only then
        // is it „übernommen“, not when `RATE_CEILING` was merely sent.
        wrap::note(ui, RichText::new("vom Gerät übernommen").small());
    } else if let Some(text) = unconfirmed(input, stored).filter(|_| fixed && input.info.is_some())
    {
        // Sent, but not (yet) named back (Gemini 5/10: no word at all).
        wrap::note(ui, RichText::new(text).small());
    }
    remeasure_button(ui, input, negotiated, measured, out);
    if changed {
        let rate = fixed.then(|| (mbit * 1e6).round().max(MIN_BITRATE_BPS as f64) as u64);
        note_save(super::super::screen_prefs::set_max_rate_bps(rate));
        out.push(ToolbarAction::MaxRate(rate));
    }
}

/// „Neu messen“: enabled for a host that measures (`negotiated`), with the
/// reason otherwise; „misst …“ until the host reports a newer measurement.
fn remeasure_button(
    ui: &mut egui::Ui,
    input: &ToolbarInput,
    negotiated: bool,
    measured: Option<Measured>,
    out: &mut Vec<ToolbarAction>,
) {
    const LABEL: &str = "Neu messen";
    wrap::reserve(ui, wrap::button_width(ui, LABEL));
    let response = ui.add_enabled(negotiated, egui::Button::new(LABEL));
    let response = match input.info {
        _ if negotiated => response.on_hover_text(
            "Das Gerät misst die Leitung neu; unter „Automatisch“ sendet es danach mit der \
             neuen Rate.",
        ),
        None => response.on_disabled_hover_text("Ohne Sitzung gibt es keine Leitung zu messen."),
        Some(info) if delivery::unknown_to_host(info) => {
            response.on_disabled_hover_text("Das Gerät misst die Leitung nicht (ältere Version).")
        }
        Some(_) => response.on_disabled_hover_text("Wartet auf die erste Statistik des Geräts."),
    };
    if response.clicked() {
        ui.data_mut(|data| data.insert_temp(remeasure_id(), Instant::now()));
        out.push(ToolbarAction::Remeasure);
    }
    let pressed = ui.data(|data| data.get_temp::<Instant>(remeasure_id()));
    if pressed.is_some_and(|at| negotiated && delivery::remeasure_pending(at.elapsed(), measured)) {
        wrap::note(ui, RichText::new("misst die Leitung …").small().italics());
    }
}
