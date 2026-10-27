//! Layout pieces of the device pages in „Geräte & Freigaben“: one card per
//! area with a clear title, a relation choice only when there is a real
//! choice, and collapsible parts for rarely used tools (2026-10-27: the
//! device menu had nested pages, one title per option and dropdowns with a
//! single entry).
use eframe::egui::{self, RichText};

use crate::app::theme;

/// A framed card with a title and an optional one-line explanation.
pub(crate) fn card<R>(
    ui: &mut egui::Ui,
    title: &str,
    hint: Option<&str>,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.add_space(8.0);
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().size(15.0));
            if let Some(hint) = hint {
                ui.label(RichText::new(hint).small().color(theme::muted(ui)));
            }
            ui.add_space(4.0);
            add(ui)
        })
        .inner
}

/// A rarely used tool inside a card, closed until the user opens it.
pub(crate) fn tool(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    title: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt(id)
        .default_open(false)
        .show(ui, add);
}

/// The relation (Direct or a room) a tool works through: buttons side by
/// side when there are several, nothing when there is only one.
pub(crate) fn relation_picker(ui: &mut egui::Ui, labels: &[&str], selected: &mut usize) {
    if labels.len() < 2 {
        *selected = 0;
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Über").color(theme::muted(ui)));
        for (index, label) in labels.iter().enumerate() {
            if ui.selectable_label(*selected == index, *label).clicked() {
                *selected = index;
            }
        }
    });
}

/// A coloured status word: green when reachable, grey otherwise.
pub(crate) fn status_chip(ui: &mut egui::Ui, text: &str, reachable: bool) {
    let color = if reachable {
        theme::success(ui)
    } else {
        theme::muted(ui)
    };
    ui.label(RichText::new(format!("● {text}")).color(color));
}

/// A refusal of the other device in words that say which of its rights is
/// missing; other texts stay as they are.
pub(crate) fn explain_remote_error(text: &str) -> String {
    if text.contains("app catalog disclosure is not authorized")
        || text.contains("no app catalog entries are granted")
    {
        return "Das andere Gerät erlaubt dir „Bildschirm steuern“ nicht. Dort unter \
                „Geräte“ → dieses Gerät → „Bildschirm- und Befehlsrechte“ \
                „Bildschirm steuern“ auf „Erlauben“ stellen; das schließt \
                Programme, Fenster und RDP ohne Nachfrage ein."
            .into();
    }
    if text.contains("root/setuid workers cannot start GUI applications")
        || text.contains("process is not in the active graphical login session")
    {
        return "Auf dem anderen Gerät läuft se nicht in einer angemeldeten \
                Desktop-Sitzung (zum Beispiel als Systemdienst vor der \
                Anmeldung). Programme startet es erst, wenn se dort in der \
                Sitzung des angemeldeten Nutzers läuft."
            .into();
    }
    if text.contains("screen-view metadata permission") {
        return "Das andere Gerät erlaubt dir „Bildschirm ansehen“ nicht. Dort unter \
                „Geräte“ → dieses Gerät → „Bildschirm- und Befehlsrechte“ auf \
                „Erlauben“ stellen („Fragen“ reicht für Listen nicht)."
            .into();
    }
    text.to_string()
}
