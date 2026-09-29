//! The "⇅ Übertragungen" window (Spec B/C): every transfer with direction,
//! source → target, state, progress, rate, remaining time once everything was
//! found, the files running right now, notes, skipped entries and errors;
//! finished ones stay until removed. It never opens by itself.
use super::prelude::*;
use super::transfer_center::{ConnectingTarget, ExternalEntry, FinishedEntry, TransferCenter};
use super::transfer_rows::{
    activity_line, counts_line, external_counts, external_issues_text, external_state,
    finished_state, issues_text, kind_icon, rate_line, running_state, tally_line, transfer_title,
};
use super::*;
use crate::app::theme;
use crate::transfer::{ActiveTransfer, TransferProgress};

const EMPTY_HINT: &str =
    "Keine Übertragungen. Strg+C merkt sich die Auswahl, Strg+V startet die Übertragung sofort.";
const CANCEL_HINT: &str = "Diese Übertragung zügig beenden; fertige Dateien bleiben";
const RESUME_HINT: &str = "Gleiche Auswahl in dieselben Zielordner; vorhandene Dateien gleicher \
                           Größe werden übersprungen, nichts wird ersetzt";
const BUTTON_HINT: &str = "Laufende und fertige Übertragungen mit Fortschritt und Fehlern";

/// What a click in the window asks for; applied after drawing.
enum RowAction {
    Cancel(usize),
    CancelAll,
    CancelConnect(u64),
    ToggleIssues(u64),
    OpenLog(String),
    Resume(u64),
    OpenTarget(u64),
    Remove(u64),
    RemoveExternal(u64),
    ClearFinished,
}

impl App {
    /// Status-bar button that opens and closes the transfer window.
    pub(in crate::app) fn ui_transfers_button(&mut self, ui: &mut egui::Ui) {
        let total = self.transfer_center.total_count();
        let label = if total == 0 {
            "⇅ Übertragungen".to_string()
        } else {
            format!("⇅ Übertragungen ({total})")
        };
        let text = if self.transfer_center.running_count() > 0 {
            RichText::new(label).color(theme::accent(ui))
        } else {
            RichText::new(label)
        };
        let open = self.transfer_center.window_open;
        if ui
            .add(egui::SelectableLabel::new(open, text))
            .on_hover_text(BUTTON_HINT)
            .clicked()
        {
            self.transfer_center.window_open = !open;
        }
    }

    pub(in crate::app) fn ui_transfer_window(&mut self, ctx: &egui::Context) {
        if !self.transfer_center.window_open {
            return;
        }
        let mut open = true;
        let mut actions = Vec::new();
        let center = &self.transfer_center;
        egui::Window::new("⇅ Übertragungen")
            .id(egui::Id::new("transfer_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([640.0, 440.0])
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(ctx.screen_rect().shrink(16.0))
            .show(ctx, |ui| window_body(ui, ctx, center, &mut actions));
        self.transfer_center.window_open = open;
        for action in actions {
            self.apply_row_action(action);
        }
    }

    fn apply_row_action(&mut self, action: RowAction) {
        match action {
            RowAction::Cancel(index) => self.transfer_center.lane.cancel(index),
            RowAction::CancelAll => self.transfer_center.lane.cancel_all(),
            RowAction::CancelConnect(id) => self.transfer_center.cancel_connecting(id),
            RowAction::ToggleIssues(id) => {
                if let Some(entry) = self.transfer_center.finished_entry_mut(id) {
                    entry.show_issues = !entry.show_issues;
                }
            }
            RowAction::OpenLog(path) => self.open_path(&path),
            RowAction::Resume(id) => {
                if let Some(job) = self.transfer_center.take_resume(id) {
                    self.submit_job(job);
                }
            }
            RowAction::OpenTarget(id) => self.open_transfer_target(id),
            RowAction::Remove(id) => self.transfer_center.remove_finished(id),
            RowAction::RemoveExternal(id) => self.transfer_center.remove_external(id),
            RowAction::ClearFinished => self.transfer_center.clear_finished(),
        }
    }
}

/// Running rows first (newest on top), then finished ones (newest on top).
fn window_body(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    center: &TransferCenter,
    actions: &mut Vec<RowAction>,
) {
    let running = center.running_count();
    let finished = center.finished.len()
        + center
            .externals
            .iter()
            .filter(|entry| entry.ended())
            .count();
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{running} laufend · {finished} fertig"));
        if center.lane.active.len() > 1 && ui.button("Alle abbrechen").clicked() {
            actions.push(RowAction::CancelAll);
        }
        if finished > 0 && ui.button("Fertige entfernen").clicked() {
            actions.push(RowAction::ClearFinished);
        }
    });
    ui.separator();
    if center.total_count() == 0 {
        ui.colored_label(theme::muted(ui), EMPTY_HINT);
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("transfer_rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for target in center.connecting.iter().rev() {
                ui.push_id(("connect", target.id), |ui| {
                    connecting_row(ui, target, actions)
                });
                ui.separator();
            }
            for (index, transfer) in center.lane.active.iter().enumerate().rev() {
                let id = center.active_id(index).unwrap_or(index as u64);
                ui.push_id(("running", id), |ui| {
                    running_row(ui, index, transfer, actions);
                });
                ui.separator();
            }
            for entry in center.externals.iter().filter(|entry| !entry.ended()) {
                external_row(ui, entry, actions);
            }
            for entry in &center.finished {
                ui.push_id(("finished", entry.id), |ui| {
                    finished_row(ui, ctx, entry, actions);
                });
                ui.separator();
            }
            for entry in center.externals.iter().filter(|entry| entry.ended()) {
                external_row(ui, entry, actions);
            }
        });
}

fn connecting_row(ui: &mut egui::Ui, target: &ConnectingTarget, actions: &mut Vec<RowAction>) {
    let title = format!(
        "{} → {}",
        target.selection.describe_source(),
        target.target_label
    );
    title_row(ui, "⇄", &title);
    ui.add(
        egui::ProgressBar::new(0.0)
            .animate(true)
            .desired_height(8.0),
    );
    ui.horizontal_wrapped(|ui| {
        let waited = target.started.elapsed().as_secs();
        ui.colored_label(theme::muted(ui), format!("verbindet… ({waited} s)"));
        if ui.small_button("Abbrechen").clicked() {
            actions.push(RowAction::CancelConnect(target.id));
        }
    });
}

fn running_row(
    ui: &mut egui::Ui,
    index: usize,
    transfer: &ActiveTransfer,
    actions: &mut Vec<RowAction>,
) {
    let title = transfer_title(&transfer.progress, transfer.job.as_deref());
    let state = running_state(&transfer.progress, transfer.canceling());
    progress_rows(ui, &transfer.progress, &title, &state);
    if !transfer.canceling()
        && ui
            .small_button("Abbrechen")
            .on_hover_text(CANCEL_HINT)
            .clicked()
    {
        actions.push(RowAction::Cancel(index));
    }
}

fn title_row(ui: &mut egui::Ui, icon: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).strong());
        ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
    });
}

/// Title, bar and the detail lines every transfer row shares.
fn progress_rows(ui: &mut egui::Ui, progress: &TransferProgress, title: &str, state: &str) {
    title_row(ui, kind_icon(progress.kind), title);
    let waiting = progress.bytes_total == 0 && progress.files_total == 0 && !progress.done;
    ui.add(
        egui::ProgressBar::new(progress.fraction())
            .animate(waiting || progress.discovering)
            .desired_height(8.0),
    );
    let mut line = format!("{state} · {}", counts_line(progress));
    if let Some(rate) = rate_line(progress) {
        line.push_str(" · ");
        line.push_str(&rate);
    }
    ui.label(RichText::new(line).small());
    if let Some(activity) = activity_line(progress) {
        let text = RichText::new(activity).small().color(theme::muted(ui));
        ui.add(egui::Label::new(text).truncate());
    }
    if let Some(note) = &progress.note {
        ui.label(RichText::new(note).small().color(theme::warning(ui)));
    }
    if let Some(tally) = tally_line(progress) {
        ui.label(RichText::new(tally).small().color(theme::muted(ui)));
    }
}

fn finished_row(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    entry: &FinishedEntry,
    actions: &mut Vec<RowAction>,
) {
    let title = transfer_title(&entry.progress, entry.job.as_deref());
    progress_rows(ui, &entry.progress, &title, finished_state(entry));
    if let Some(failure) = &entry.failure {
        ui.colored_label(theme::danger(ui), failure);
    }
    let has_issues = !entry.issues.is_empty() || !entry.errors.is_empty();
    ui.horizontal_wrapped(|ui| {
        if has_issues {
            let label = if entry.show_issues {
                "Fehler ausblenden"
            } else {
                "Fehler anzeigen"
            };
            if ui.small_button(label).clicked() {
                actions.push(RowAction::ToggleIssues(entry.id));
            }
        }
        if entry.can_resume()
            && ui
                .small_button("Fehlende übertragen")
                .on_hover_text(RESUME_HINT)
                .clicked()
        {
            actions.push(RowAction::Resume(entry.id));
        }
        if entry.job.is_some() && ui.small_button("Zielordner öffnen").clicked() {
            actions.push(RowAction::OpenTarget(entry.id));
        }
        if ui.small_button("Entfernen").clicked() {
            actions.push(RowAction::Remove(entry.id));
        }
    });
    if entry.show_issues && has_issues {
        issue_list(ui, ctx, entry, actions);
    }
}

/// The complete error list with "Alle Fehler kopieren" and "Protokoll öffnen".
fn issue_list(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    entry: &FinishedEntry,
    actions: &mut Vec<RowAction>,
) {
    let text = issues_text(entry);
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Alle Fehler kopieren").clicked() {
            ctx.copy_text(text.clone());
        }
        if let Some(log) = &entry.progress.log_path {
            if ui
                .small_button("Protokoll öffnen")
                .on_hover_text(log.as_str())
                .clicked()
            {
                actions.push(RowAction::OpenLog(log.clone()));
            }
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("issues")
        .max_height(160.0)
        .show(ui, |ui| {
            let mut shown = text.as_str();
            ui.add(
                egui::TextEdit::multiline(&mut shown)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(ui.available_width())
                    .desired_rows(1),
            );
        });
}

fn external_row(ui: &mut egui::Ui, entry: &ExternalEntry, actions: &mut Vec<RowAction>) {
    let snapshot = &entry.snapshot;
    ui.push_id(("external", snapshot.id), |ui| {
        title_row(ui, "📋", &snapshot.label);
        let fraction = if snapshot.files_total > 0 {
            (snapshot.files_done as f32 / snapshot.files_total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ui.add(
            egui::ProgressBar::new(fraction)
                .animate(!entry.ended() && snapshot.files_total == 0)
                .desired_height(8.0),
        );
        let line = format!("{} · {}", external_state(entry), external_counts(snapshot));
        ui.label(RichText::new(line).small());
        if let Some(note) = &snapshot.note {
            ui.label(RichText::new(note).small().color(theme::warning(ui)));
        }
        ui.horizontal_wrapped(|ui| {
            if !snapshot.issues.is_empty() && ui.small_button("Alle Fehler kopieren").clicked() {
                ui.ctx().copy_text(external_issues_text(snapshot));
            }
            if entry.ended() && ui.small_button("Entfernen").clicked() {
                actions.push(RowAction::RemoveExternal(snapshot.id));
            }
        });
    });
    ui.separator();
}
