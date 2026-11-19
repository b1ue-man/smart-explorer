//! The codec rows of the viewer's toolbar (plan 2026-11-14 „Bildkette“
//! 4.10, BK6): „Codec: Automatisch / H.264 / HEVC / AV1 / Verlustfrei“ (it
//! replaces the switch „Verlustfrei“ and keeps its meaning), „Dekodierer:
//! Automatisch / Hardware / Software“ and „Höchste Datenrate“ (B-09, moved
//! here from `screen_toolbar_ui.rs`). Codec and decoder are stored at once
//! (`screen_prefs::set_codec_choice`); a codec choice that changes the
//! negotiation goes out as `SET_DECODERS` to a host that announced it (P1,
//! `ViewerSession::set_decoders`) and opens the session again otherwise, a
//! decoder choice takes over at the next key frame when a decoder of it
//! decodes the running stream (`ViewerSession::switch_decoder`, V1) and
//! goes out as `SET_DECODERS` as well,
//! „Verlustfrei“ switches at once, a new data rate goes out as
//! `RATE_CEILING`.
//!
//! P9 (plan 2026-11-17): each closed list names the choice and what the
//! running session applies (`screen_toolbar_state.rs`): „Codec:
//! Automatisch (AV1)“, „Dekodierer: Hardware (wird angewendet …)“ until the
//! session answered, „Codec: H.264 → läuft: AV1“ when it runs otherwise.
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use eframe::egui::{self, RichText, WidgetText};

use super::super::screen_prefs::{CodecSetting, DecoderPreference};
use super::applied::{self, Applied, PendingChoices, SessionFacts, Setting};
use super::{ToolbarAction, ToolbarInput};

/// The last failed save of a toolbar choice (review 2026-11-19: the errors
/// were dropped, a choice looked stored and was gone after a restart).
static SAVE_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Keeps the outcome of storing a choice: an error stays shown in the
/// toolbar until a later save succeeds.
pub(crate) fn note_save(result: std::io::Result<()>) {
    let mut slot = SAVE_ERROR.lock().unwrap_or_else(PoisonError::into_inner);
    *slot = result.err().map(|error| error.to_string());
}

/// „Einstellung nicht gespeichert“ with the reason, while the last save failed.
pub(super) fn save_error_row(ui: &mut egui::Ui) {
    let error = SAVE_ERROR
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(error) = error {
        let text = format!("Einstellung nur für diese Sitzung, nicht gespeichert: {error}");
        let warning = crate::app::theme::warning(ui);
        super::wrap::note(ui, RichText::new(text).color(warning));
    }
}

/// The facts of the running session the lists compare with.
fn session_facts(ui: &egui::Ui, input: &ToolbarInput) -> SessionFacts {
    let info = input.info;
    SessionFacts {
        codec: info
            .and_then(|info| info.stream.as_ref())
            .map(|stream| stream.codec),
        decoder: info.and_then(|info| info.decoder.clone()),
        direct_showing: super::super::screen_direct::showing(ui.ctx()),
        pictures: info.map_or(0, |info| info.pictures),
    }
}

/// The changes of this window that wait for the session (egui's memory).
fn with_pending<R>(ui: &egui::Ui, edit: impl FnOnce(&mut PendingChoices) -> R) -> R {
    let id = egui::Id::new("screen_toolbar_pending");
    let mut pending = ui
        .data(|data| data.get_temp::<PendingChoices>(id))
        .unwrap_or_default();
    let result = edit(&mut pending);
    ui.data_mut(|data| data.insert_temp(id, pending));
    result
}

/// Whether `setting` waits for the session now.
fn waits(ui: &egui::Ui, setting: Setting, facts: &SessionFacts, now: Instant) -> bool {
    with_pending(ui, |pending| pending.pending(setting, facts, now))
}

/// `setting` changed now.
fn changed(ui: &egui::Ui, setting: Setting, facts: &SessionFacts, now: Instant) {
    with_pending(ui, |pending| pending.start(setting, facts, now));
}

/// The closed list's text; one that runs otherwise in the warning colour.
fn list_text(ui: &egui::Ui, name: &str, chosen: &str, state: &Applied) -> WidgetText {
    let text = RichText::new(applied::list_text(name, chosen, state));
    match state {
        Applied::Other(_) => text.color(crate::app::theme::warning(ui)).into(),
        Applied::Pending => text.italics().into(),
        Applied::Unknown | Applied::Applied(_) => text.into(),
    }
}

/// „Codec“ and „Dekodierer“ as two drop-down lists.
pub(super) fn codec_rows(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let facts = session_facts(ui, input);
    let now = Instant::now();
    let codec = super::super::screen_prefs::codec_setting();
    let state = applied::codec_state(codec, &facts, waits(ui, Setting::Codec, &facts, now));
    let mut chosen = codec;
    let _ = super::wrap::item(ui, "codec", |ui| {
        ({
            let text = list_text(ui, "Codec", codec.label(), &state);
            super::wrap::fitted(ui, egui::ComboBox::from_id_salt("screen_codec"), text)
        })
        .show_ui(ui, |ui| {
            super::wrap::popup(ui, CodecSetting::ALL.map(CodecSetting::label));
            for setting in CodecSetting::ALL {
                ui.selectable_value(&mut chosen, setting, setting.label());
            }
        })
        .response
        .on_hover_text(codec_hint())
    });
    if chosen != codec {
        out.push(ToolbarAction::Codec(chosen));
        changed(ui, Setting::Codec, &facts, now);
    }
    let decoder = super::super::screen_prefs::decoder_choice();
    let state = applied::decoder_state(decoder, &facts, waits(ui, Setting::Decoder, &facts, now));
    let mut picked = decoder;
    let _ = super::wrap::item(ui, "decoder", |ui| {
        ({
            let text = list_text(ui, "Dekodierer", decoder.label(), &state);
            super::wrap::fitted(ui, egui::ComboBox::from_id_salt("screen_decoder"), text)
        })
        .show_ui(ui, |ui| {
            super::wrap::popup(ui, DecoderPreference::ALL.map(DecoderPreference::label));
            for choice in DecoderPreference::ALL {
                ui.selectable_value(&mut picked, choice, choice.label());
            }
        })
        .response
        .on_hover_text("Automatisch: Hardware, bei Fehlern Software.")
    });
    if picked != decoder {
        out.push(ToolbarAction::Decoder(picked));
        changed(ui, Setting::Decoder, &facts, now);
    }
    output_rows(ui, &facts, now, out);
}

/// The hover text of „Codec“: the mode „Scharf (GPU)“ (plan 2026-11-17 P9)
/// is named once the list offers it (`CodecSetting` with key `sharp`).
fn codec_hint() -> String {
    use crate::screen::core::sharp_mode::{SHARP_HINT, SHARP_KEY};
    let mut hint = String::from(
        "Automatisch wählt nach Hardware beider Geräte, AV1 nie zuerst; \
         Verlustfrei überträgt Kacheln.",
    );
    if CodecSetting::parse(SHARP_KEY).is_some() {
        hint.push('\n');
        hint.push_str(SHARP_HINT);
    }
    hint
}

/// „Dekodierer-Bibliothek: Automatisch / FFmpeg / Media Foundation“ and
/// „Anzeige: Automatisch / Direct3D 11 / wgpu“ (plan 2026-11-17 P8), on
/// Windows, where both libraries and both displays exist.
#[cfg(windows)]
fn output_rows(
    ui: &mut egui::Ui,
    facts: &SessionFacts,
    now: Instant,
    out: &mut Vec<ToolbarAction>,
) {
    use super::super::screen_prefs::{DecoderLibrary, DisplayChoice};
    let library = super::super::screen_prefs::decoder_library_choice();
    let state = applied::library_state(library, facts, waits(ui, Setting::Library, facts, now));
    let mut chosen = library;
    let _ = super::wrap::item(ui, "library", |ui| {
        ({
            let text = list_text(ui, "Bibliothek", library.label(), &state);
            super::wrap::fitted(
                ui,
                egui::ComboBox::from_id_salt("screen_decoder_library"),
                text,
            )
        })
        .show_ui(ui, |ui| {
            super::wrap::popup(ui, DecoderLibrary::ALL.map(DecoderLibrary::label));
            for choice in DecoderLibrary::ALL {
                ui.selectable_value(&mut chosen, choice, choice.label());
            }
        })
        .response
        .on_hover_text(
            "Automatisch: erst Hardware (FFmpeg D3D11VA/DXVA2, dann Media Foundation), \
             danach Software.",
        )
    });
    if chosen != library {
        out.push(ToolbarAction::DecoderLibrary(chosen));
        changed(ui, Setting::Library, facts, now);
    }
    let display = super::super::screen_prefs::display_choice();
    let state = applied::display_state(display, facts, waits(ui, Setting::Display, facts, now));
    let mut picked = display;
    let _ = super::wrap::item(ui, "display", |ui| {
        ({
            let text = list_text(ui, "Anzeige", display.label(), &state);
            super::wrap::fitted(ui, egui::ComboBox::from_id_salt("screen_display"), text)
        })
        .show_ui(ui, |ui| {
            super::wrap::popup(ui, DisplayChoice::ALL.map(DisplayChoice::label));
            for choice in DisplayChoice::ALL {
                ui.selectable_value(&mut picked, choice, choice.label());
            }
        })
        .response
        .on_hover_text("Automatisch: Direct3D 11 ohne CPU-Umweg, sonst wgpu.")
    });
    if picked != display {
        out.push(ToolbarAction::Display(picked));
        changed(ui, Setting::Display, facts, now);
    }
}

/// One library and one display elsewhere: nothing to choose.
#[cfg(not(windows))]
fn output_rows(
    _ui: &mut egui::Ui,
    _facts: &SessionFacts,
    _now: Instant,
    _out: &mut Vec<ToolbarAction>,
) {
}

/// Stores a „Codec“ or „Dekodierer“ choice in `prefs` and the process
/// (`set_codec_choice`); `true` when the session must change its decoders:
/// `SET_DECODERS` where the host takes it (P1), a new decoder choice also
/// tries `ViewerSession::switch_decoder`; otherwise the session opens
/// again. „Verlustfrei“ alone switches by `SET_MODE`, as the old switch did
/// (4.10).
pub fn apply_codec(
    action: &ToolbarAction,
    prefs: &mut super::super::screen_prefs::ViewerPrefs,
) -> bool {
    let set = super::super::screen_prefs::set_codec_choice;
    let again = match action {
        ToolbarAction::Codec(codec) => {
            // „Verlustfrei“ offers the tiles alone (`chosen_decoders`), so it
            // and „Automatisch“ differ in the offers too (Gemini 8).
            let offers = |setting: CodecSetting| (setting.preference(), setting.lossless());
            let before = offers(prefs.codec_setting());
            note_save(set(Some(*codec), None));
            prefs.codec = Some(*codec);
            prefs.lossless = codec.lossless();
            before != offers(*codec)
        }
        ToolbarAction::Decoder(decoder) => {
            note_save(set(None, Some(*decoder)));
            prefs.decoder = Some(*decoder);
            true
        }
        // P8/P9: another library takes over at the next key frame when one
        // of its decoders decodes the running stream (`switch_decoder`) and
        // goes out as `SET_DECODERS` with the register's offers of now;
        // another display takes effect at the next picture
        // (`screen_direct.rs`), also after a presenter that failed.
        ToolbarAction::DecoderLibrary(library) => {
            note_save(super::super::screen_prefs::set_output_choice(
                Some(*library),
                None,
            ));
            prefs.decoder_library = Some(*library);
            true
        }
        ToolbarAction::Display(display) => {
            note_save(super::super::screen_prefs::set_output_choice(
                None,
                Some(*display),
            ));
            prefs.display = Some(*display);
            false
        }
        _ => return false,
    };
    if let Err(error) = prefs.save() {
        note_save(Err(error));
    }
    again
}

/// „Höchste Datenrate“ (B-09): automatic, or a ceiling in Mbit/s, stored at
/// once (`screen_prefs::set_max_rate_bps`) and sent to the host. A new
/// ceiling starts with the session's current target.
/// The width of the „Höchste Datenrate“ field in points: a typed value
/// scrolls inside it (17a: egui's `DragValue` grew with the typed text, past
/// the toolbar's edge).
const RATE_FIELD_WIDTH: f32 = 70.0;
const RATE_UNIT: &str = "Mbit/s";

/// The field of „Höchste Datenrate“: the stored value while it is not typed
/// into, the typed text while it is; `Some` with the value in Mbit/s once a
/// parsable text is left (Enter or focus lost; a German comma is the decimal
/// point). Every key stroke went out as `RATE_CEILING` before (Gemini 10).
/// Where the field's error waits for the next try (`rate_field`).
fn rate_error_id() -> egui::Id {
    egui::Id::new("screen_max_rate_error")
}

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
        .desired_width(RATE_FIELD_WIDTH)
        .clip_text(true);
    let edited = ui.add(field);
    super::wrap::note_editing(ui, &edited);
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
    // The largest ceiling a `u64` of bit/s holds (Codex 7/17: 1e30 went out
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

/// Whether the host reports `stored` as the ceiling it applies
/// (`STATS` → `ChainReport::rate_ceiling_kbps`, sent once a second by a
/// host with the picture chain report). The report counts whole kbit/s, so
/// it proves only a ceiling of whole kbit/s (Codex 5/23: a report of the
/// value before, rounded alike, confirmed a new one); finer ceilings are
/// not called „übernommen“.
fn confirmed(input: &ToolbarInput, stored: Option<u64>) -> bool {
    let reported = input
        .info
        .and_then(|info| info.stats.as_ref())
        .and_then(|stats| stats.chain.as_ref())
        .and_then(|chain| chain.rate_ceiling_kbps);
    stored.is_some_and(|bps| bps % 1000 == 0 && reported == Some(bps / 1000))
}

/// Why a sent ceiling `stored` is not called „übernommen“ (`confirmed`).
fn unconfirmed(input: &ToolbarInput, stored: Option<u64>) -> Option<&'static str> {
    let bps = stored?;
    let reported = input
        .info
        .and_then(|info| info.stats.as_ref())
        .and_then(|stats| stats.chain.as_ref())
        .and_then(|chain| chain.rate_ceiling_kbps);
    Some(match reported {
        None => "gesendet; noch keine Bestätigung des Geräts",
        Some(_) if bps % 1000 != 0 => "gesendet; das Gerät bestätigt nur ganze kbit/s",
        Some(_) => "gesendet; das Gerät meldet noch eine andere Obergrenze",
    })
}

pub(super) fn max_rate_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    use crate::screen::core::rate_control::MIN_BITRATE_BPS;
    let stored = super::super::screen_prefs::max_rate_bps();
    let current = input.info.and_then(|info| info.stats.as_ref());
    let start = stored.or_else(|| current.map(|stats| stats.target_kbps * 1000));
    let mut mbit = start.unwrap_or(MIN_BITRATE_BPS).max(MIN_BITRATE_BPS) as f64 / 1e6;
    let mut limited = stored.is_some();
    // Switch, field and unit stay in one row (Gemini 5): widths from egui's
    // own sizing (`wrap::checkbox_width`, `text_edit_width`).
    let label = "Höchste Datenrate";
    let gap = ui.spacing().item_spacing.x;
    // What is drawn this pass is measured (Codex 5/21: the unticked switch
    // broke its row for a field it did not draw); the field of a switch
    // ticked in this pass follows in the next one, measured with it.
    let shown = stored.is_some();
    let mut group = super::wrap::checkbox_width(ui, label);
    if shown {
        group += gap + super::wrap::text_edit_width(RATE_FIELD_WIDTH);
        group += gap + super::wrap::label_width(ui, RATE_UNIT);
    }
    super::wrap::reserve(ui, group);
    let mut changed = super::wrap::checkbox(ui, &mut limited, label)
        .on_hover_text("Ohne Haken: automatisch. Wirkt sofort, wenn das Gerät es unterstützt.")
        .changed();
    if !limited {
        // Unticked: no field, no error of it (H6).
        ui.data_mut(|data| data.remove::<String>(rate_error_id()));
    }
    if limited && shown {
        // Field and unit break the row themselves when the group was wider
        // than the bar (Gemini 5/8).
        super::wrap::reserve(ui, super::wrap::text_edit_width(RATE_FIELD_WIDTH));
        if let Some(typed) = rate_field(ui, mbit) {
            let typed = typed.max(MIN_BITRATE_BPS as f64 / 1e6);
            changed |= typed != mbit;
            mbit = typed;
        }
        super::wrap::reserve(ui, super::wrap::label_width(ui, RATE_UNIT));
        ui.label(RATE_UNIT);
        // After the unit, an entry of its own (Codex 6/21: between field
        // and unit it tore the unit away from its number).
        let error = ui.data(|data| data.get_temp::<String>(rate_error_id()));
        if let Some(error) = error.as_deref() {
            let warning = crate::app::theme::warning(ui);
            super::wrap::note(ui, RichText::new(error).color(warning));
        }
    }
    // P9: what the session does with the ceiling.
    if limited
        && input.info.is_some_and(|info| {
            info.rate_ceiling == crate::daemon::RateCeilingState::UnsupportedByHost
        })
    {
        let warning = crate::app::theme::warning(ui);
        super::wrap::note(
            ui,
            RichText::new("vom Gerät nicht unterstützt").color(warning),
        );
    } else if limited && confirmed(input, stored) {
        // The host's own statistics name this ceiling (Codex 3/22): only
        // then is it „übernommen“, not when `RATE_CEILING` was merely sent.
        super::wrap::note(ui, RichText::new("vom Gerät übernommen").small());
    } else if let Some(text) =
        unconfirmed(input, stored).filter(|_| limited && input.info.is_some())
    {
        // Sent, but not (yet) named back (Gemini 5/10: no word at all).
        super::wrap::note(ui, RichText::new(text).small());
    }
    if changed {
        let rate = limited.then(|| (mbit * 1e6).round().max(MIN_BITRATE_BPS as f64) as u64);
        note_save(super::super::screen_prefs::set_max_rate_bps(rate));
        out.push(ToolbarAction::MaxRate(rate));
    }
}
