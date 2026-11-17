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
use eframe::egui;

use super::super::screen_prefs::{CodecSetting, DecoderPreference};
use super::{ToolbarAction, ToolbarInput};

/// „Codec“ and „Dekodierer“ as two drop-down lists.
pub(super) fn codec_rows(ui: &mut egui::Ui, out: &mut Vec<ToolbarAction>) {
    let codec = super::super::screen_prefs::codec_setting();
    let mut chosen = codec;
    let _ = egui::ComboBox::from_id_salt("screen_codec")
        .selected_text(format!("Codec: {}", codec.label()))
        .show_ui(ui, |ui| {
            for setting in CodecSetting::ALL {
                ui.selectable_value(&mut chosen, setting, setting.label());
            }
        })
        .response
        .on_hover_text(
            "Automatisch wählt nach Hardware beider Geräte; Verlustfrei überträgt Kacheln.",
        );
    if chosen != codec {
        out.push(ToolbarAction::Codec(chosen));
    }
    let decoder = super::super::screen_prefs::decoder_choice();
    let mut picked = decoder;
    let _ = egui::ComboBox::from_id_salt("screen_decoder")
        .selected_text(format!("Dekodierer: {}", decoder.label()))
        .show_ui(ui, |ui| {
            for choice in DecoderPreference::ALL {
                ui.selectable_value(&mut picked, choice, choice.label());
            }
        })
        .response
        .on_hover_text("Automatisch: Hardware, bei Fehlern Software.");
    if picked != decoder {
        out.push(ToolbarAction::Decoder(picked));
    }
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
    if changed {
        let rate = limited.then(|| (mbit * 1e6).round().max(MIN_BITRATE_BPS as f64) as u64);
        let _ = super::super::screen_prefs::set_max_rate_bps(rate);
        out.push(ToolbarAction::MaxRate(rate));
    }
}
