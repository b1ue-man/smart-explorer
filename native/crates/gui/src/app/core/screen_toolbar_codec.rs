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
use std::time::Instant;

use eframe::egui::{self, RichText, WidgetText};

use super::super::screen_prefs::{CodecSetting, DecoderPreference};
use super::applied::{self, Applied, PendingChoices, SessionFacts, Setting};
use super::{ToolbarAction, ToolbarInput};

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
    let _ = egui::ComboBox::from_id_salt("screen_codec")
        .selected_text(list_text(ui, "Codec", codec.label(), &state))
        .show_ui(ui, |ui| {
            for setting in CodecSetting::ALL {
                ui.selectable_value(&mut chosen, setting, setting.label());
            }
        })
        .response
        .on_hover_text(codec_hint());
    if chosen != codec {
        out.push(ToolbarAction::Codec(chosen));
        changed(ui, Setting::Codec, &facts, now);
    }
    let decoder = super::super::screen_prefs::decoder_choice();
    let state = applied::decoder_state(decoder, &facts, waits(ui, Setting::Decoder, &facts, now));
    let mut picked = decoder;
    let _ = egui::ComboBox::from_id_salt("screen_decoder")
        .selected_text(list_text(ui, "Dekodierer", decoder.label(), &state))
        .show_ui(ui, |ui| {
            for choice in DecoderPreference::ALL {
                ui.selectable_value(&mut picked, choice, choice.label());
            }
        })
        .response
        .on_hover_text("Automatisch: Hardware, bei Fehlern Software.");
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
    let _ = egui::ComboBox::from_id_salt("screen_decoder_library")
        .selected_text(list_text(ui, "Bibliothek", library.label(), &state))
        .show_ui(ui, |ui| {
            for choice in DecoderLibrary::ALL {
                ui.selectable_value(&mut chosen, choice, choice.label());
            }
        })
        .response
        .on_hover_text(
            "Automatisch: erst Hardware (FFmpeg D3D11VA/DXVA2, dann Media Foundation), \
             danach Software.",
        );
    if chosen != library {
        out.push(ToolbarAction::DecoderLibrary(chosen));
        changed(ui, Setting::Library, facts, now);
    }
    let display = super::super::screen_prefs::display_choice();
    let state = applied::display_state(display, facts, waits(ui, Setting::Display, facts, now));
    let mut picked = display;
    let _ = egui::ComboBox::from_id_salt("screen_display")
        .selected_text(list_text(ui, "Anzeige", display.label(), &state))
        .show_ui(ui, |ui| {
            for choice in DisplayChoice::ALL {
                ui.selectable_value(&mut picked, choice, choice.label());
            }
        })
        .response
        .on_hover_text("Automatisch: Direct3D 11 ohne CPU-Umweg, sonst wgpu.");
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
            let before = prefs.codec_setting().preference();
            let _ = set(Some(*codec), None);
            prefs.codec = Some(*codec);
            prefs.lossless = codec.lossless();
            before != codec.preference()
        }
        ToolbarAction::Decoder(decoder) => {
            let _ = set(None, Some(*decoder));
            prefs.decoder = Some(*decoder);
            true
        }
        // P8/P9: another library takes over at the next key frame when one
        // of its decoders decodes the running stream (`switch_decoder`) and
        // goes out as `SET_DECODERS` with the register's offers of now;
        // another display takes effect at the next picture
        // (`screen_direct.rs`), also after a presenter that failed.
        ToolbarAction::DecoderLibrary(library) => {
            let _ = super::super::screen_prefs::set_output_choice(Some(*library), None);
            prefs.decoder_library = Some(*library);
            true
        }
        ToolbarAction::Display(display) => {
            let _ = super::super::screen_prefs::set_output_choice(None, Some(*display));
            prefs.display = Some(*display);
            false
        }
        _ => return false,
    };
    let _ = prefs.save();
    again
}

/// „Höchste Datenrate“ (B-09): automatic, or a ceiling in Mbit/s, stored at
/// once (`screen_prefs::set_max_rate_bps`) and sent to the host. A new
/// ceiling starts with the session's current target.
pub(super) fn max_rate_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    use crate::screen::core::rate_control::MIN_BITRATE_BPS;
    let stored = super::super::screen_prefs::max_rate_bps();
    let current = input.info.and_then(|info| info.stats.as_ref());
    let start = stored.or_else(|| current.map(|stats| stats.target_kbps * 1000));
    let mut mbit = start.unwrap_or(MIN_BITRATE_BPS).max(MIN_BITRATE_BPS) as f64 / 1e6;
    let mut limited = stored.is_some();
    let mut changed = ui
        .checkbox(&mut limited, "Höchste Datenrate")
        .on_hover_text("Ohne Haken: automatisch. Wirkt sofort, wenn das Gerät es unterstützt.")
        .changed();
    if limited {
        // One percent of the value per dragged pixel.
        let speed = mbit / 100.0;
        let field = egui::DragValue::new(&mut mbit)
            .range(MIN_BITRATE_BPS as f64 / 1e6..=f64::MAX)
            .speed(speed)
            .suffix(" Mbit/s");
        changed |= ui.add(field).changed();
    }
    // P9: what the session does with the ceiling.
    if limited
        && input.info.is_some_and(|info| {
            info.rate_ceiling == crate::daemon::RateCeilingState::UnsupportedByHost
        })
    {
        ui.colored_label(
            crate::app::theme::warning(ui),
            "vom Gerät nicht unterstützt",
        );
    }
    if changed {
        let rate = limited.then(|| (mbit * 1e6).round().max(MIN_BITRATE_BPS as f64) as u64);
        let _ = super::super::screen_prefs::set_max_rate_bps(rate);
        out.push(ToolbarAction::MaxRate(rate));
    }
}
