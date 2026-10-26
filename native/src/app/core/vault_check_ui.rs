//! The page „Prüfung“ of the vault properties (U29-8, T21): „Tresor prüfen“
//! asks the worker for the findings of one unlocked site and offers the
//! action of each finding. A finding that removes cipher files asks first
//! (the worker keeps a copy of what it removes). The findings hold names of
//! the vault, so a lock drops them.
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use crossbeam_channel::Receiver;
use eframe::egui;

use super::vault_feed::{guard, poll, spawn, Poll};
use super::vault_models::{failure_text, Failure};
use super::vault_models_compare::{buttons_for, finding_title, removed_root, ButtonModel};
use super::vault_sidebar_ui::Effect;
use crate::daemon::CompareItemWire;

type CheckAnswer = Result<Vec<CompareItemWire>, Failure>;
type ActionAnswer = (String, Result<(), Failure>);

#[derive(Default)]
struct Check {
    rx: Option<Receiver<CheckAnswer>>,
    items: Option<Vec<CompareItemWire>>,
    error: Option<String>,
    row_errors: HashMap<String, String>,
    done: HashSet<String>,
    action_rx: Option<Receiver<ActionAnswer>>,
    /// An action waiting for the answer to its question: (item, button).
    asking: Option<(String, ButtonModel)>,
}

fn state() -> &'static Mutex<HashMap<String, Check>> {
    static STATE: OnceLock<Mutex<HashMap<String, Check>>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

fn failure(error: crate::daemon::VaultIpcError) -> Failure {
    Failure {
        code: error.code,
        message: error.message,
    }
}

/// A lock drops the findings of the site (they hold names of the vault).
pub(crate) fn on_locked(site: &str) {
    if let Some(check) = guard(state()).get_mut(site) {
        check.items = None;
        check.asking = None;
        check.row_errors.clear();
        check.done.clear();
    }
}

/// The site left the list.
pub(crate) fn close_site(site: &str) {
    guard(state()).remove(site);
}

fn start_check(ctx: &egui::Context, site: &str, check: &mut Check) {
    check.error = None;
    check.row_errors.clear();
    check.done.clear();
    let target = site.to_string();
    match spawn(ctx, "vault-check", move || -> CheckAnswer {
        crate::daemon::vault_check(&target).map_err(failure)
    }) {
        Ok(rx) => check.rx = Some(rx),
        Err(error) => check.error = Some(error),
    }
}

fn start_action(ctx: &egui::Context, site: &str, check: &mut Check, item: &str, key: &str) {
    let (target, item_id, action) = (site.to_string(), item.to_string(), key.to_string());
    match spawn(ctx, "vault-check-fix", move || -> ActionAnswer {
        let answer = crate::daemon::vault_resolve(&target, Some(&item_id), &action, false, true)
            .map(|_| ())
            .map_err(failure);
        (item_id, answer)
    }) {
        Ok(rx) => check.action_rx = Some(rx),
        Err(error) => check.error = Some(error),
    }
}

/// Draws the page for `site`; `unlocked` is the state of the site now.
pub(crate) fn page(ui: &mut egui::Ui, site: &str, unlocked: bool) -> Vec<Effect> {
    let ctx = ui.ctx().clone();
    let mut effects = Vec::new();
    let mut held = guard(state());
    let check = held.entry(site.to_string()).or_default();
    let mut rerun = false;
    if let Poll::Ready(answer) = poll(&mut check.rx) {
        match answer {
            Ok(items) => check.items = Some(items),
            Err(failure) => check.error = Some(failure_text(&failure)),
        }
    }
    match poll(&mut check.action_rx) {
        Poll::Ready((item, Ok(()))) => {
            check.done.insert(item);
            rerun = true;
        }
        Poll::Ready((item, Err(failure))) => {
            check.row_errors.insert(item, failure_text(&failure));
        }
        Poll::Gone => check.error = Some("Die Aktion endete ohne Antwort.".to_string()),
        Poll::Pending => {}
    }
    let busy = check.rx.is_some() || check.action_rx.is_some();
    if rerun {
        start_check(&ctx, site, check);
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(unlocked && !busy, egui::Button::new("Tresor prüfen"))
            .clicked()
        {
            start_check(&ctx, site, check);
        }
        if busy {
            ui.spinner();
            ui.label("Wird geprüft …");
        }
    });
    if !unlocked {
        ui.label("Zum Prüfen muss der Tresor entsperrt sein.");
        if ui.button("Entsperren …").clicked() {
            effects.push(Effect::AskUnlock {
                site: site.to_string(),
                then_open: false,
            });
        }
    }
    if let Some(error) = &check.error {
        ui.colored_label(crate::app::theme::danger(ui), error);
    }
    let removed = removed_root();
    let mut pressed: Option<(String, ButtonModel)> = None;
    if let Some(items) = &check.items {
        if items.is_empty() {
            ui.label("Keine Auffälligkeiten gefunden.");
        }
        for item in items {
            ui.group(|ui| {
                let done = check.done.contains(&item.item);
                ui.label(egui::RichText::new(finding_title(item)).strong());
                if !item.path.is_empty() {
                    ui.label(&item.path);
                }
                if let Some(note) = &item.note {
                    ui.small(note);
                }
                if let Some(error) = check.row_errors.get(&item.item) {
                    ui.colored_label(crate::app::theme::danger(ui), error);
                }
                if done {
                    ui.small("erledigt");
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    for button in buttons_for(item, &removed) {
                        if ui
                            .add_enabled(unlocked && !busy, egui::Button::new(button.title))
                            .clicked()
                        {
                            pressed = Some((item.item.clone(), button));
                        }
                    }
                });
            });
        }
    }
    if let Some((item, button)) = pressed {
        if button.question.is_some() {
            check.asking = Some((item, button));
        } else {
            start_action(&ctx, site, check, &item, &button.key);
        }
    }
    let mut answer: Option<bool> = None;
    if let Some((_, button)) = &check.asking {
        crate::app::dialog_geometry::window(&ctx, "vault_check_ui.0", button.title)
            .id(egui::Id::new("vault_check_ask"))
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(&ctx, |ui| {
                ui.label(button.question.as_deref().unwrap_or_default());
                ui.horizontal(|ui| {
                    if ui.button("Ja, ausführen").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Abbrechen").clicked() {
                        answer = Some(false);
                    }
                });
            });
    }
    match answer {
        Some(true) => {
            if let Some((item, button)) = check.asking.take() {
                start_action(&ctx, site, check, &item, &button.key);
            }
        }
        Some(false) => check.asking = None,
        None => {}
    }
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
    }
    effects
}
