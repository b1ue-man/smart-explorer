//! The window „Tresor vergleichen – {Jobname}“ (U29-8, T18 to T20): the
//! comparison mode of a vault sync job. A conflict is never resolved by the
//! sync itself; after opening, this window offers the comparison. The state,
//! the threads (`vault-compare`, `vault-resolve`, `vault-merge`) and the
//! questions are here; the list and the detail are drawn in
//! `vault_compare_list_ui.rs` and `vault_compare_detail_ui.rs`.
//!
//! Every action that removes something asks first; after an action the list
//! is compared anew. The entries hold names of the vault, so a lock of one of
//! the two sites drops them at once.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use crossbeam_channel::Receiver;
use eframe::egui;

use super::vault_compare_detail_ui::DetailState;
use super::vault_feed::{self, guard, poll, spawn, Poll};
use super::vault_models::{failure_text, Failure, RowState};
use super::vault_models_compare::{GroupFilter, RowModel};
use super::vault_sidebar_ui::Effect;
use crate::daemon::CompareViewFull;

type CompareAnswer = Result<CompareViewFull, Failure>;

/// What an action thread reports.
struct ActionDone {
    /// The entry the action was for (`None`: the collective action).
    item: Option<String>,
    result: Result<crate::daemon::ResolveResult, Failure>,
}

/// A question waiting for the answer.
pub(super) enum Ask {
    Item {
        item: String,
        key: String,
        text: String,
    },
    Bulk {
        keep_a: bool,
        text: String,
    },
}

/// What the rows were made from: a new view, filter or `done` set remakes them.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct RowsKey {
    revision: u64,
    items: usize,
    filter: GroupFilter,
    done: usize,
}

pub(super) struct Window {
    pub(super) job: String,
    pub(super) name: String,
    /// The two sites of the job (side A, side B).
    pub(super) sites: [String; 2],
    pub(super) filter: GroupFilter,
    /// Whether the last request asked for `structure`.
    pub(super) requested_structure: bool,
    pub(super) view: Option<Arc<CompareViewFull>>,
    rx: Option<Receiver<CompareAnswer>>,
    pub(super) error: Option<String>,
    pub(super) locked: bool,
    pub(super) selected: Option<String>,
    pub(super) done: HashSet<String>,
    pub(super) row_errors: HashMap<String, String>,
    action_rx: Option<Receiver<ActionDone>>,
    pub(super) ask: Option<Ask>,
    pub(super) detail: DetailState,
    /// Compare again as soon as the running action is over.
    recompare: bool,
    pub(super) rows: Arc<Vec<RowModel>>,
    rows_key: Option<RowsKey>,
}

impl Window {
    /// A comparison or an action runs.
    pub(super) fn busy(&self) -> bool {
        self.rx.is_some() || self.action_rx.is_some()
    }

    /// Whether the comparison itself runs.
    pub(super) fn comparing(&self) -> bool {
        self.rx.is_some()
    }

    /// Makes the rows again when the view, the filter or the `done` set changed.
    pub(super) fn refresh_rows(&mut self) {
        let Some(view) = &self.view else {
            self.rows = Arc::default();
            self.rows_key = None;
            return;
        };
        let key = RowsKey {
            revision: view.head.revision,
            items: view.items.len(),
            filter: self.filter,
            done: self.done.len(),
        };
        if self.rows_key.as_ref() != Some(&key) {
            self.rows = Arc::new(super::vault_models_compare::compare_rows(
                &view.items,
                &self.filter,
                &self.done,
            ));
            self.rows_key = Some(key);
        }
    }
}

fn state() -> &'static Mutex<Option<Window>> {
    static STATE: OnceLock<Mutex<Option<Window>>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

fn failure(error: crate::daemon::VaultIpcError) -> Failure {
    Failure {
        code: error.code,
        message: error.message,
    }
}

impl Window {
    fn new(job: &str, name: &str, sites: [String; 2]) -> Self {
        Self {
            job: job.to_string(),
            name: name.to_string(),
            sites,
            filter: GroupFilter::default(),
            requested_structure: false,
            view: None,
            rx: None,
            error: None,
            locked: false,
            selected: None,
            done: HashSet::new(),
            row_errors: HashMap::new(),
            action_rx: None,
            ask: None,
            detail: DetailState::default(),
            recompare: false,
            rows: Arc::default(),
            rows_key: None,
        }
    }
}

/// Opens the comparison of the vault job `job` and starts it.
pub(crate) fn open(ctx: &egui::Context, job: &str, name: &str) {
    let sites = match crate::vault::find_vault_job(job) {
        Ok(found) => [found.vault_site_a, found.vault_site_b],
        Err(_) => [String::new(), String::new()],
    };
    let mut window = Window::new(job, name, sites);
    start_compare(ctx, &mut window);
    *guard(state()) = Some(window);
}

/// The window with a comparison at hand (the render test of the harness; no
/// request goes to the service).
#[cfg(test)]
pub(crate) fn open_for_test(
    job: &str,
    name: &str,
    sites: [&str; 2],
    view: CompareViewFull,
    selected: Option<&str>,
) {
    let mut window = Window::new(job, name, sites.map(str::to_string));
    window.view = Some(Arc::new(view));
    window.selected = selected.map(str::to_string);
    *guard(state()) = Some(window);
}

/// Closes the window of the test.
#[cfg(test)]
pub(crate) fn close_for_test() {
    *guard(state()) = None;
}

/// A site was locked: the entries go when the window concerns it.
pub(crate) fn on_locked(site: &str) {
    if let Some(window) = guard(state()).as_mut() {
        if window.sites.iter().any(|known| known == site) {
            drop_entries(window);
        }
    }
}

fn drop_entries(window: &mut Window) {
    window.view = None;
    window.rows = Arc::default();
    window.selected = None;
    window.ask = None;
    window.done.clear();
    window.row_errors.clear();
    window.detail.clear();
    window.locked = true;
    window.error = Some("Der Tresor ist gesperrt.".to_string());
}

/// Starts a comparison with the groups the filter asks for.
pub(super) fn start_compare(ctx: &egui::Context, window: &mut Window) {
    window.error = None;
    window.locked = false;
    window.done.clear();
    window.row_errors.clear();
    window.requested_structure = window.filter.requested().len() > 1;
    let job = window.job.clone();
    let groups = window.filter.requested();
    match spawn(ctx, "vault-compare", move || -> CompareAnswer {
        crate::daemon::vault_compare(&job, &groups).map_err(failure)
    }) {
        Ok(rx) => window.rx = Some(rx),
        Err(error) => window.error = Some(error),
    }
}

/// One action on `item` (or, without one, the collective action); the user
/// answered the question it asks, so the worker gets `confirmed`.
pub(super) fn start_action(
    ctx: &egui::Context,
    window: &mut Window,
    item: Option<String>,
    key: &str,
) {
    let (job, key, all) = (window.job.clone(), key.to_string(), item.is_none());
    let target = item.clone();
    match spawn(ctx, "vault-resolve", move || ActionDone {
        result: crate::daemon::vault_resolve(&job, target.as_deref(), &key, all, true)
            .map_err(failure),
        item: target,
    }) {
        Ok(rx) => window.action_rx = Some(rx),
        Err(error) => window.error = Some(error),
    }
}

/// „Ergebnis speichern“ of the merge view.
pub(super) fn start_merge(ctx: &egui::Context, window: &mut Window, item: String, text: String) {
    let job = window.job.clone();
    match spawn(ctx, "vault-merge", move || ActionDone {
        result: crate::daemon::vault_merge_apply(&job, &item, &text)
            .map(|()| crate::daemon::ResolveResult {
                item: Some(item.clone()),
                resolved: 1,
                failed: Vec::new(),
            })
            .map_err(failure),
        item: Some(item),
    }) {
        Ok(rx) => window.action_rx = Some(rx),
        Err(error) => window.error = Some(error),
    }
}

/// Takes the answers of the threads.
fn collect(ctx: &egui::Context, window: &mut Window) {
    if let Poll::Ready(answer) = poll(&mut window.rx) {
        match answer {
            Ok(view) => {
                window.error = view.head.unavailable.clone();
                window.view = Some(Arc::new(view));
            }
            Err(failure) => {
                window.locked = failure.code.as_deref() == Some("vault_locked");
                window.view = None;
                window.error = Some(failure_text(&failure));
            }
        }
    }
    match poll(&mut window.action_rx) {
        Poll::Ready(done) => {
            match done.result {
                Ok(result) => {
                    if let Some(item) = &done.item {
                        window.done.insert(item.clone());
                    }
                    for failed in &result.failed {
                        window
                            .row_errors
                            .insert(failed.item.clone(), failed.error.clone());
                    }
                }
                Err(failure) => {
                    let text = failure_text(&failure);
                    match &done.item {
                        Some(item) => {
                            window.row_errors.insert(item.clone(), text);
                        }
                        None => window.error = Some(text),
                    }
                }
            }
            window.recompare = true;
        }
        Poll::Gone => {
            window.error = Some("Die Aktion endete ohne Antwort des Dienstes.".to_string());
            window.recompare = true;
        }
        Poll::Pending => {}
    }
    if window.recompare && !window.busy() {
        window.recompare = false;
        let errors = std::mem::take(&mut window.row_errors);
        let done = std::mem::take(&mut window.done);
        start_compare(ctx, window);
        // The refreshed list shows the finished entries and their errors until
        // the new comparison arrives.
        window.row_errors = errors;
        window.done = done;
    }
}

/// Draws the window and returns what it asks of the main window.
pub(crate) fn show(ctx: &egui::Context) -> Vec<Effect> {
    let mut held = guard(state());
    let Some(window) = held.as_mut() else {
        return Vec::new();
    };
    let rows = vault_feed::rows();
    collect(ctx, window);
    // A missed lock event: no side is unlocked any more.
    let any_unlocked = window.sites.iter().any(|site| {
        rows.iter()
            .any(|row| &row.site == site && row.state == RowState::Unlocked)
    });
    if window.view.is_some() && !any_unlocked {
        drop_entries(window);
    }
    window.refresh_rows();
    let mut effects = Vec::new();
    let mut open = true;
    egui::Window::new(format!("Tresor vergleichen – {}", window.name))
        .id(egui::Id::new("vault_compare"))
        .open(&mut open)
        .collapsible(false)
        .default_size([980.0, 560.0])
        .show(ctx, |ui| {
            super::vault_compare_list_ui::content(ui, window, &rows, &mut effects);
        });
    show_question(ctx, window);
    if window.busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
    }
    if !open {
        *held = None;
    }
    effects
}

/// The question before a removing action (U29-8).
fn show_question(ctx: &egui::Context, window: &mut Window) {
    let Some(ask) = &window.ask else {
        return;
    };
    let (title, text) = match ask {
        Ask::Item { text, .. } => ("Aktion ausführen?", text.clone()),
        Ask::Bulk { text, .. } => ("Alle Konflikte auflösen?", text.clone()),
    };
    let mut answer: Option<bool> = None;
    egui::Window::new(title)
        .id(egui::Id::new("vault_compare_ask"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(&text);
            ui.horizontal(|ui| {
                if ui.button("Ja, ausführen").clicked() {
                    answer = Some(true);
                }
                if ui.button("Abbrechen").clicked() {
                    answer = Some(false);
                }
            });
        });
    match (answer, window.ask.take()) {
        (Some(true), Some(Ask::Item { item, key, .. })) => {
            start_action(ctx, window, Some(item), &key);
        }
        (Some(true), Some(Ask::Bulk { keep_a, .. })) => {
            start_action(ctx, window, None, if keep_a { "keep-a" } else { "keep-b" });
        }
        (None, ask) => window.ask = ask,
        _ => {}
    }
}
