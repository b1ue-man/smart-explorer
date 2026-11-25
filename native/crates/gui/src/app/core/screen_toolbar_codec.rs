//! The codec rows of the viewer's toolbar (plan 2026-11-14 „Bildkette“
//! 4.10, BK6): „Codec: Automatisch / H.264 / HEVC / AV1 / Verlustfrei“ (it
//! replaces the switch „Verlustfrei“ and keeps its meaning) and „Dekodierer:
//! Automatisch / Hardware / Software“; „Datenrate“ (formerly „Höchste
//! Datenrate“) is in `screen_toolbar_rate.rs`. Codec and decoder are stored at once
//! (`screen_prefs::set_codec_choice`); a codec choice that changes the
//! negotiation goes out as `SET_DECODERS` to a host that announced it (P1,
//! `ViewerSession::set_decoders`) and opens the session again otherwise, a
//! decoder choice takes over at the next key frame when a decoder of it
//! decodes the running stream (`ViewerSession::switch_decoder`, V1) and
//! goes out as `SET_DECODERS` as well,
//! „Verlustfrei“ switches at once.
//!
//! P9 (plan 2026-11-17): each closed list names the choice and what the
//! running session applies (`screen_toolbar_state.rs`): „Codec:
//! Automatisch (AV1)“, „Dekodierer: Hardware (wird angewendet …)“ until the
//! session answered, „Codec: H.264 → läuft: AV1“ when it runs otherwise.
//!
//! User rule 2026-11-24 („was du anzeigst MUSS funktionieren“): an entry
//! this pair of devices cannot run stays in its list, disabled, with the
//! reason as hover text (`screen_prefs::codec_offer`); a stored choice the
//! host cannot serve reads „Codec: AV1 → Automatisch (H.264)“ with the
//! reason below, the session runs with „Automatisch“.
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use eframe::egui::{self, RichText, WidgetText};

use super::super::screen_prefs::{codec_offer, CodecSetting, DecoderPreference};
use super::applied::{self, chosen_text, Applied, Pair, PendingChoices, SessionFacts, Setting};
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

/// One list of the bar: an entry with a reason is shown disabled with it as
/// hover text. The entry picked, when another than `current`.
fn list<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    salt: &str,
    text: WidgetText,
    current: T,
    entries: &[(T, &str, Option<String>)],
    hint: &str,
) -> Option<T> {
    let mut picked = current;
    let _ = super::wrap::item(ui, salt, |ui| {
        super::wrap::fitted(ui, egui::ComboBox::from_id_salt(salt), text)
            .show_ui(ui, |ui| {
                super::wrap::popup(ui, entries.iter().map(|(_, label, _)| *label));
                for (value, label, reason) in entries {
                    let Some(reason) = reason else {
                        ui.selectable_value(&mut picked, *value, *label);
                        continue;
                    };
                    let entry = egui::SelectableLabel::new(picked == *value, *label);
                    let _ = ui
                        .add_enabled(false, entry)
                        .on_disabled_hover_text(format!("{label}: {reason}"));
                }
            })
            .response
            .on_hover_text(hint)
    });
    (picked != current).then_some(picked)
}

/// `(value, label, reason)` of each entry.
fn labelled<T: Copy>(
    entries: Vec<(T, Option<String>)>,
    label: fn(T) -> &'static str,
) -> Vec<(T, &'static str, Option<String>)> {
    entries
        .into_iter()
        .map(|(value, reason)| (value, label(value), reason))
        .collect()
}

/// „Codec“ and „Dekodierer“ as two drop-down lists.
pub(super) fn codec_rows(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let facts = session_facts(ui, input);
    let now = Instant::now();
    let pair = applied::pair(ui, input);
    let session = &pair.session;
    let codec = super::super::screen_prefs::codec_setting();
    let state = applied::codec_state(
        session.codec,
        &facts,
        waits(ui, Setting::Codec, &facts, now),
    );
    let text = list_text(
        ui,
        "Codec",
        &chosen_text(codec.label(), session.codec.label()),
        &state,
    );
    let entries = labelled(pair.codecs(), CodecSetting::label);
    if let Some(chosen) = list(ui, "screen_codec", text, codec, &entries, &codec_hint()) {
        out.push(ToolbarAction::Codec(chosen));
        changed(ui, Setting::Codec, &facts, now);
    }
    let decoder = super::super::screen_prefs::decoder_choice();
    let state = applied::decoder_state(decoder, &facts, waits(ui, Setting::Decoder, &facts, now));
    let chosen = chosen_text(decoder.label(), session.decoder.label());
    let text = list_text(ui, "Dekodierer", &chosen, &state);
    let entries = labelled(pair.decoders(), DecoderPreference::label);
    let hint = "Automatisch: Hardware, bei Fehlern Software.";
    if let Some(picked) = list(ui, "screen_decoder", text, decoder, &entries, hint) {
        out.push(ToolbarAction::Decoder(picked));
        changed(ui, Setting::Decoder, &facts, now);
    }
    output_rows(ui, &facts, &pair, now, out);
    // Why this session does not run a stored choice (user rule 2026-11-24).
    let warning = crate::app::theme::warning(ui);
    for note in &session.notes {
        super::wrap::note(ui, RichText::new(note).small().color(warning));
    }
    // A host that refused a stored choice whose encoders were not known at
    // the opening: once they are, the session opens again with what it
    // serves, once per refusal.
    let Some(end) = input.info.and_then(|info| info.end.as_ref()) else {
        return;
    };
    let id = egui::Id::new("screen_codec_reopened");
    let done = ui.data(|data| data.get_temp::<String>(id)).as_deref() == Some(end.text.as_str());
    let refused = applied::refused_for_choice(end.code.as_deref(), codec, session.codec);
    if refused && !done && codec_offer::enabled() {
        ui.data_mut(|data| data.insert_temp(id, end.text.clone()));
        out.push(ToolbarAction::Device(input.current.clone()));
    }
}

/// The hover text of „Codec“: the mode „Scharf (GPU)“ (plan 2026-11-17 P9)
/// is named once the list offers it (`CodecSetting` with key `sharp`).
fn codec_hint() -> String {
    use crate::screen::core::sharp_mode::{SHARP_HINT, SHARP_KEY};
    let mut hint = String::from(
        "Automatisch wählt nach Hardware beider Geräte, AV1 nie zuerst; \
         Verlustfrei überträgt Kacheln. Grau: mit diesem Gerät nicht möglich.",
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
    pair: &Pair,
    now: Instant,
    out: &mut Vec<ToolbarAction>,
) {
    use super::super::screen_prefs::{DecoderLibrary, DisplayChoice};
    let (library, display) = super::super::screen_prefs::stored_output();
    let state = applied::library_state(library, facts, waits(ui, Setting::Library, facts, now));
    let chosen = chosen_text(library.label(), pair.library_running().label());
    let text = list_text(ui, "Bibliothek", &chosen, &state);
    let entries = labelled(pair.libraries(), DecoderLibrary::label);
    let hint = "Automatisch: erst Hardware (FFmpeg D3D11VA/DXVA2, dann Media Foundation), \
                danach Software.";
    if let Some(chosen) = list(ui, "screen_decoder_library", text, library, &entries, hint) {
        out.push(ToolbarAction::DecoderLibrary(chosen));
        changed(ui, Setting::Library, facts, now);
    }
    let in_force = super::super::screen_prefs::display_choice();
    let state = applied::display_state(in_force, facts, waits(ui, Setting::Display, facts, now));
    let text = list_text(
        ui,
        "Anzeige",
        &chosen_text(display.label(), in_force.label()),
        &state,
    );
    let entries = labelled(applied::displays(), DisplayChoice::label);
    let hint = "Automatisch: Direct3D 11 ohne CPU-Umweg, sonst wgpu.";
    if let Some(picked) = list(ui, "screen_display", text, display, &entries, hint) {
        out.push(ToolbarAction::Display(picked));
        changed(ui, Setting::Display, facts, now);
    }
}

/// One library and one display elsewhere: nothing to choose.
#[cfg(not(windows))]
fn output_rows(
    _ui: &mut egui::Ui,
    _facts: &SessionFacts,
    _pair: &Pair,
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
