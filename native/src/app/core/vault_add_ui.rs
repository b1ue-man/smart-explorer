//! The window „Tresor hinzufügen…“ (U29-6): enter a vault that already
//! exists. The location is checked without a password (`detect_location`);
//! the window says „Tresor Format n, …“ or „Kein Tresor an diesem Ort“ and
//! enters the vault only when it is one se can open.
use std::sync::{Mutex, OnceLock};

use crossbeam_channel::Receiver;
use eframe::egui;

use super::vault_create_ui::{check_location, location_row, resolve, Checked, Chooser};
use super::vault_feed::{guard, poke, poll, spawn, Poll};
use super::vault_models_manage::{add_summary, label_issue};
use super::vault_sidebar_ui::{Effect, PickTarget};
use crate::creds::SavedConnection;

fn state() -> &'static Mutex<Option<AddDialog>> {
    static STATE: OnceLock<Mutex<Option<AddDialog>>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

struct AddDialog {
    location: String,
    chooser: Chooser,
    label: String,
    /// The location the last check was for, and what it found.
    found: Option<(String, Checked)>,
    check_rx: Option<Receiver<Checked>>,
    checked_for: String,
    auto_check: bool,
    error: Option<String>,
    add_rx: Option<Receiver<Result<String, String>>>,
}

/// Opens „Tresor hinzufügen…“, with a location when the hint bar of a
/// folder that holds a vault asked for it.
pub(crate) fn open(prefill: Option<String>) {
    let auto_check = prefill.is_some();
    *guard(state()) = Some(AddDialog {
        location: prefill.unwrap_or_default(),
        chooser: Chooser::default(),
        label: String::new(),
        found: None,
        check_rx: None,
        checked_for: String::new(),
        auto_check,
        error: None,
        add_rx: None,
    });
}

/// A folder picked for the window.
pub(crate) fn picked(folder: String) {
    if let Some(add) = guard(state()).as_mut() {
        add.location = folder;
        add.auto_check = true;
    }
}

pub(crate) fn show(ctx: &egui::Context, connections: &[SavedConnection]) -> Vec<Effect> {
    let mut effects = Vec::new();
    let mut held = guard(state());
    show_add(ctx, &mut held, connections, &mut effects);
    effects
}

fn show_add(
    ctx: &egui::Context,
    state: &mut Option<AddDialog>,
    connections: &[SavedConnection],
    effects: &mut Vec<Effect>,
) {
    let Some(add) = state.as_mut() else {
        return;
    };
    if let Poll::Ready(found) = poll(&mut add.check_rx) {
        add.found = Some((add.checked_for.clone(), found));
    }
    let mut closed = false;
    match poll(&mut add.add_rx) {
        Poll::Ready(Ok(label)) => {
            effects.push(Effect::Notice(format!(
                "Tresor „{label}“ wurde eingetragen."
            )));
            closed = true;
        }
        Poll::Ready(Err(error)) => add.error = Some(error),
        Poll::Gone => add.error = Some("Das Eintragen endete ohne Antwort.".to_string()),
        Poll::Pending => {}
    }
    if closed {
        poke();
        *state = None;
        return;
    }
    let busy = add.check_rx.is_some() || add.add_rx.is_some();
    let current = add.location.trim().to_string();
    let verdict = match &add.found {
        Some((for_location, found)) if *for_location == current => Some(found.clone()),
        _ => None,
    };
    let (mut check, mut enter, mut cancel) = (add.auto_check && !busy, false, false);
    crate::app::dialog_geometry::window(ctx, "vault_add_ui.0", "Tresor hinzufügen")
        .id(egui::Id::new("vault_add_window"))
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label("Wo liegt der Tresor?");
            location_row(
                ui,
                &mut add.location,
                &mut add.chooser,
                connections,
                PickTarget::AddLocation,
                effects,
                !busy,
            );
            let can_add = match &verdict {
                Some(Ok(found)) => {
                    let (text, can_add) = add_summary(found);
                    ui.label(text);
                    can_add
                }
                Some(Err(problem)) => {
                    ui.colored_label(crate::app::theme::danger(ui), problem);
                    false
                }
                None => false,
            };
            ui.label("Bezeichnung");
            ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut add.label)
                    .hint_text("leer: Name des Ordners")
                    .desired_width(280.0),
            );
            if let Some(issue) = (!add.label.trim().is_empty())
                .then(|| label_issue(&add.label))
                .flatten()
            {
                ui.colored_label(crate::app::theme::danger(ui), issue);
            }
            if let Some(error) = &add.error {
                ui.colored_label(crate::app::theme::danger(ui), error);
            }
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Bitte warten …");
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                let has_location = !add.location.trim().is_empty();
                if ui
                    .add_enabled(!busy && has_location, egui::Button::new("Prüfen"))
                    .clicked()
                {
                    check = true;
                }
                if ui
                    .add_enabled(!busy && can_add, egui::Button::new("Eintragen"))
                    .clicked()
                {
                    enter = true;
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Abbrechen"))
                    .clicked()
                {
                    cancel = true;
                }
            });
        });
    if cancel {
        *state = None;
        return;
    }
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        return;
    }
    add.auto_check = false;
    if check && !current.is_empty() {
        add.error = None;
        add.checked_for = current.clone();
        let location = current.clone();
        match spawn(ctx, "vault-detect", move || check_location(location)) {
            Ok(rx) => add.check_rx = Some(rx),
            Err(error) => add.error = Some(error),
        }
    } else if enter {
        add.error = None;
        let label = (!add.label.trim().is_empty()).then(|| add.label.trim().to_string());
        let started = spawn(ctx, "vault-add", move || {
            crate::vault::add_site(
                &crate::vault::RegistryStore::open_default(),
                &current,
                label.as_deref(),
                &resolve,
            )
            .map(|record| record.display_label().to_string())
            .map_err(|error| error.to_string())
        });
        match started {
            Ok(rx) => add.add_rx = Some(rx),
            Err(error) => add.error = Some(error),
        }
    }
}
