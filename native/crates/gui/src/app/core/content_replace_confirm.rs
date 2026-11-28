//! The confirmation of „In Dateien ersetzen“ (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C „Bestätigung“): a
//! dry run counts the places first, then the window names the folder, the
//! filter and the number of places and files, as Notepad++ („Are you
//! sure?“, Abbrechen preselected) and VS Code („Replace N occurrences across
//! M files with 'X'?“) do. Nothing is written before „Ersetzen“.
use crate::app::content_search_results::group_thousands;
use crate::app::prelude::*;

use super::content_replace_results::{JobSelection, ReplaceJob};

/// What the dry run found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct PreviewTotals {
    pub(in crate::app) files_searched: u64,
    pub(in crate::app) files_changed: u64,
    pub(in crate::app) replacements: u64,
    /// (relative path, German reason) of the files a run would skip.
    pub(in crate::app) skipped: Vec<(String, String)>,
}

/// Where the dry run stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum PreviewState {
    /// Started from the results panel: „Ersetzen durch“ is asked first.
    Asking(String),
    /// Places and files counted so far.
    Running {
        files: u64,
        replacements: u64,
    },
    Ready(PreviewTotals),
    Failed(String),
}

/// The open confirmation of one job.
pub(in crate::app) struct ReplaceConfirm {
    pub(in crate::app) job: ReplaceJob,
    pub(in crate::app) state: PreviewState,
    /// The first frame of „Ready“ puts the keyboard focus on „Abbrechen“.
    focused: bool,
}

/// What the window asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum ConfirmAction {
    None,
    /// Count the places with this replacement text.
    Preview(String),
    Apply,
    Cancel,
}

/// Key of the window (its layer id).
pub(in crate::app) const CONFIRM_KEY: &str = "content_search.replace_confirm";

fn places_word(count: u64) -> &'static str {
    if count == 1 {
        "Stelle"
    } else {
        "Stellen"
    }
}

/// The text of the confirmation, one line each.
pub(in crate::app) fn confirm_lines(job: &ReplaceJob, totals: &PreviewTotals) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(match job.selection {
        JobSelection::All => "Wirklich alle Vorkommen ersetzen in:".to_string(),
        JobSelection::Hits(_) => "Wirklich die ausgewählten Treffer ersetzen in:".to_string(),
    });
    lines.push(job.root.clone());
    lines.push(format!("auf: {}", job.place.label));
    let filters = job.query.filters.trim();
    lines.push(format!(
        "Für Dateitypen: {}",
        if filters.is_empty() { "*.*" } else { filters }
    ));
    if job.query.respect_ignore_files {
        lines.push("Ignorierdateien (.gitignore) werden beachtet.".to_string());
    }
    let places = format!(
        "{} {} in {} {}",
        group_thousands(totals.replacements),
        places_word(totals.replacements),
        group_thousands(totals.files_changed),
        crate::app::content_search_results::files_word(totals.files_changed)
    );
    lines.push(if job.replacement.is_empty() {
        format!("{places} löschen (Ersetzen durch nichts)?")
    } else {
        format!("{places} durch „{}“ ersetzen?", job.replacement)
    });
    if !totals.skipped.is_empty() {
        lines.push(format!(
            "{} übersprungen (Gründe unten).",
            group_thousands(totals.skipped.len() as u64)
        ));
    }
    lines.push(
        "Vor dem Überschreiben wird jede Datei gesichert; „Rückgängig“ im Ergebnis stellt sie \
         wieder her."
            .to_string(),
    );
    lines
}

impl ReplaceConfirm {
    /// The confirmation of `job`; it asks for the replacement text until
    /// `previewing` starts the dry run.
    pub(in crate::app) fn asking(job: ReplaceJob) -> Self {
        let text = job.replacement.clone();
        Self {
            job,
            state: PreviewState::Asking(text),
            focused: false,
        }
    }

    pub(in crate::app) fn window_id() -> egui::Id {
        egui::Id::new(CONFIRM_KEY)
    }

    /// Moves to the dry run of `text` (the focus goes to „Abbrechen“ once
    /// the counts are there).
    pub(in crate::app) fn previewing(&mut self, text: String) {
        self.job.replacement = text;
        self.state = PreviewState::Running {
            files: 0,
            replacements: 0,
        };
        self.focused = false;
    }

    /// The window; Escape cancels (handled by the caller's keys).
    pub(in crate::app) fn show(&mut self, ctx: &egui::Context) -> ConfirmAction {
        let mut action = ConfirmAction::None;
        let mut open = true;
        egui::Window::new("Wirklich ersetzen?")
            .id(Self::window_id())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| action = self.ui(ui));
        if !open {
            action = ConfirmAction::Cancel;
        }
        action
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> ConfirmAction {
        let mut action = ConfirmAction::None;
        match &mut self.state {
            PreviewState::Asking(text) => {
                let chosen = self.job.chosen_count().unwrap_or(0);
                ui.label(format!(
                    "{} ausgewählte {} in {} ersetzen.",
                    group_thousands(chosen),
                    places_word(chosen),
                    self.job.root
                ));
                ui.horizontal(|ui| {
                    ui.label("Ersetzen durch:");
                    let edit = ui
                        .add(egui::TextEdit::singleline(text).desired_width(280.0))
                        .on_hover_text(crate::app::content_search_dialog::REPLACE_TIP);
                    if !std::mem::replace(&mut self.focused, true) {
                        edit.request_focus();
                    }
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        action = ConfirmAction::Preview(text.clone());
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Weiter …").clicked() {
                        action = ConfirmAction::Preview(text.clone());
                    }
                    if ui.button("Abbrechen").clicked() {
                        action = ConfirmAction::Cancel;
                    }
                });
            }
            PreviewState::Running {
                files,
                replacements,
            } => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!(
                        "Vorschau läuft: {} {} in {} {} …",
                        group_thousands(*replacements),
                        places_word(*replacements),
                        group_thousands(*files),
                        crate::app::content_search_results::files_word(*files)
                    ));
                });
                if ui.button("Abbrechen").clicked() {
                    action = ConfirmAction::Cancel;
                }
            }
            PreviewState::Failed(error) => {
                ui.colored_label(crate::app::theme::danger(ui), error.as_str());
                if ui.button("Schließen").clicked() {
                    action = ConfirmAction::Cancel;
                }
            }
            PreviewState::Ready(totals) => {
                let totals = totals.clone();
                action = self.ui_ready(ui, &totals);
            }
        }
        action
    }

    fn ui_ready(&mut self, ui: &mut egui::Ui, totals: &PreviewTotals) -> ConfirmAction {
        let mut action = ConfirmAction::None;
        if totals.replacements == 0 {
            ui.label("Keine Stelle zu ersetzen.");
        } else {
            for (index, line) in confirm_lines(&self.job, totals).iter().enumerate() {
                if index == 1 {
                    ui.strong(line);
                } else {
                    ui.label(line);
                }
            }
        }
        ui.label(
            RichText::new(format!(
                "{} {} durchsucht.",
                group_thousands(totals.files_searched),
                crate::app::content_search_results::files_word(totals.files_searched)
            ))
            .color(crate::app::theme::muted(ui)),
        );
        if !totals.skipped.is_empty() {
            egui::CollapsingHeader::new(format!(
                "Übersprungen ({})",
                group_thousands(totals.skipped.len() as u64)
            ))
            .id_salt("replace_confirm_skipped")
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for (rel, reason) in &totals.skipped {
                            ui.label(format!("{rel} – {reason}"));
                        }
                    });
            });
        }
        ui.separator();
        ui.horizontal(|ui| {
            if totals.replacements == 0 {
                if ui.button("Schließen").clicked() {
                    action = ConfirmAction::Cancel;
                }
                return;
            }
            if ui.button("Ersetzen").clicked() {
                action = ConfirmAction::Apply;
            }
            let cancel = ui.button("Abbrechen");
            // Abbrechen is preselected (Notepad++ MB_DEFBUTTON2): Enter on
            // the focused button cancels.
            if !std::mem::replace(&mut self.focused, true) {
                cancel.request_focus();
            }
            if cancel.clicked() {
                action = ConfirmAction::Cancel;
            }
        });
        action
    }
}
