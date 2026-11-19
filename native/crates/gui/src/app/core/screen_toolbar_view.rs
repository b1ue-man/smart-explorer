//! The view and settings items of the viewer's toolbar (fit, full screen,
//! latency, statistics, sound, codec, decoder, data rate, monitor and
//! resolution), each one unit of the row (`wrap::item`).
use eframe::egui::{self, RichText};

use super::super::screen_prefs::Fit;
use super::{codec_rows, display_rows, wrap, ToolbarAction, ToolbarInput};

pub(super) fn view_row(ui: &mut egui::Ui, input: &ToolbarInput, out: &mut Vec<ToolbarAction>) {
    let fit = input.prefs.fit;
    wrap::item(ui, "fit", |ui| {
        if ui.selectable_label(fit == Fit::Fit, "Einpassen").clicked() && fit != Fit::Fit {
            out.push(ToolbarAction::Fit(Fit::Fit));
        }
        if ui.selectable_label(fit == Fit::Actual, "1:1").clicked() && fit != Fit::Actual {
            out.push(ToolbarAction::Fit(Fit::Actual));
        }
    });
    let fullscreen = wrap::item(ui, "fullscreen", |ui| {
        ui.selectable_label(input.fullscreen, "Vollbild")
            .on_hover_text(
                "Vollbild verlassen: Strg+Alt+Pause oder Strg+Alt+Umschalt+F; F11 schaltet \
                 Vollbild um.",
            )
            .clicked()
    });
    if fullscreen {
        out.push(ToolbarAction::Fullscreen);
    }
    let mut low_latency = input.prefs.low_latency;
    let latency_changed = wrap::item(ui, "low_latency", |ui| {
        ui.checkbox(&mut low_latency, "Niedrigste Latenz (Bildrisse möglich)")
            .changed()
    });
    if latency_changed {
        // P9: the Direct3D 11 display presents with it from its next picture.
        super::super::screen_present::set_low_latency(low_latency);
        out.push(ToolbarAction::LowLatency(low_latency));
    }
    let direct = super::super::screen_direct::showing(ui.ctx());
    if let Some(note) =
        super::super::screen_present::latency_note(input.low_latency_changed, direct)
    {
        wrap::item(ui, "latency_note", |ui| {
            ui.label(RichText::new(note).small());
        });
    }
    let mut stats = input.stats_on;
    if wrap::item(ui, "stats", |ui| {
        ui.checkbox(&mut stats, "Statistik").changed()
    }) {
        out.push(ToolbarAction::Stats(stats));
    }
    wrap::item(ui, "audio", |ui| {
        super::super::screen_audio::toolbar_row(ui, input.info, out);
    });
    // „Codec“ (replaces „Verlustfrei“), „Dekodierer“, „Höchste Datenrate“.
    codec_rows::codec_rows(ui, input, out);
    wrap::item(ui, "max_rate", |ui| {
        codec_rows::max_rate_row(ui, input, out)
    });
    // „Monitor“ and „Auflösung“ of the host (plan 2026-11-17 P5).
    display_rows::display_rows(ui, input, out);
    codec_rows::save_error_row(ui);
}
