//! „In Dateien ersetzen“ on the desktop (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, M3): the flow from the
//! dialog or the results panel through the dry run and the confirmation to
//! the run, whose record shows in the results panel with its totals, the
//! skipped files with their reasons and „Rückgängig“.
use crate::agent_proto::{ContentQuery, ContentSummary};
use crate::app::content_search_results::{FileHits, RecordState, SearchRecord};
use crate::app::content_search_results_ui::PanelAction;
use crate::app::prelude::*;
use crate::app::App;

use super::content_replace_confirm::{ConfirmAction, PreviewState, PreviewTotals, ReplaceConfirm};
use super::content_replace_results::{JobSelection, ReplaceJob, UndoState};
use super::content_replace_run::{ReplaceRun, ReplaceUpdate, RunTotals, UndoRun};
use super::{KeyOwner, TextPlace, TextTools};

/// The replacement state of the text tools.
#[derive(Default)]
pub(in crate::app) struct ReplaceTools {
    /// The open confirmation (asking, counting or ready).
    pub(in crate::app) confirm: Option<ReplaceConfirm>,
    /// The dry run of the confirmation and what it found so far.
    preview: Option<ReplaceRun>,
    preview_found: PreviewTotals,
    /// Runs that write; each fills its record.
    pub(in crate::app) runs: Vec<ReplaceRun>,
    undos: Vec<UndoRun>,
}

impl ReplaceTools {
    /// A dry run, a run or a restore is going on.
    pub(in crate::app) fn running(&self) -> bool {
        self.preview.is_some() || !self.runs.is_empty() || !self.undos.is_empty()
    }

    /// Stops the writing runs after their current file (the backups of the
    /// files already written stay restorable).
    pub(in crate::app) fn cancel_runs(&self) {
        for run in &self.runs {
            run.cancel();
        }
    }

    /// Closes the confirmation; its dry run stops.
    pub(in crate::app) fn close_confirm(&mut self) {
        if let Some(run) = self.preview.take() {
            run.cancel();
        }
        self.confirm = None;
    }

    /// The ids of the records whose run still writes.
    pub(in crate::app) fn busy_records(&self) -> Vec<u64> {
        self.runs.iter().map(|run| run.id).collect()
    }
}

fn summary_of(totals: &RunTotals) -> ContentSummary {
    ContentSummary {
        files_searched: totals.files_searched,
        files_matched: totals.files_changed,
        hits: totals.replacements,
        skipped: totals.skipped,
        ..ContentSummary::default()
    }
}

impl TextTools {
    fn replace_busy(&self) -> Result<(), String> {
        if self.replace.confirm.is_some() || !self.replace.runs.is_empty() {
            return Err("Ein Ersetzen läuft bereits; bitte erst beenden oder abbrechen.".into());
        }
        Ok(())
    }

    /// „In Dateien ersetzen“ of the dialog: every match below `root`.
    pub(in crate::app) fn replace_all(
        &mut self,
        query: ContentQuery,
        root: String,
        place: TextPlace,
        replacement: String,
    ) -> Result<(), String> {
        self.request_replace(ReplaceJob {
            query,
            root,
            place,
            replacement,
            selection: JobSelection::All,
            source: None,
        })
    }

    /// A job whose text is known: the dry run starts at once.
    pub(in crate::app) fn request_replace(&mut self, job: ReplaceJob) -> Result<(), String> {
        self.replace_busy()?;
        let text = job.replacement.clone();
        self.replace.confirm = Some(ReplaceConfirm::asking(job));
        self.start_preview(text);
        Ok(())
    }

    /// „Ersetzen…“ of the panel: the confirmation asks the text first.
    pub(in crate::app) fn ask_replace(&mut self, job: ReplaceJob) -> Result<(), String> {
        self.replace_busy()?;
        self.replace.confirm = Some(ReplaceConfirm::asking(job));
        Ok(())
    }

    fn start_preview(&mut self, text: String) {
        self.dialog.remember_replacement_text(&text);
        let Some(confirm) = &mut self.replace.confirm else {
            return;
        };
        confirm.previewing(text);
        self.replace.preview_found = PreviewTotals::default();
        match ReplaceRun::start(0, &confirm.job, true) {
            Ok(run) => self.replace.preview = Some(run),
            Err(error) => confirm.state = PreviewState::Failed(error),
        }
    }

    /// „Ersetzen“ in the confirmation: the run and its record.
    fn apply_replace(&mut self) {
        let Some(confirm) = self.replace.confirm.take() else {
            return;
        };
        let job = confirm.job;
        let id = self.results.next_id();
        match ReplaceRun::start(id, &job, false) {
            Ok(run) => {
                self.results
                    .begin(SearchRecord::new_replace(id, &job), false);
                self.replace.runs.push(run);
                self.panel.visible = true;
                self.panel.cursor = None;
                self.panel.selection.clear();
                self.panel.notice = None;
            }
            Err(error) => self.panel.notice = Some(error),
        }
    }

    /// „Rückgängig“ of the record `search`.
    pub(in crate::app) fn undo_replace(&mut self, search: u64) {
        let Some(record) = self.results.record_mut(search) else {
            return;
        };
        let Some(info) = record.replace.as_mut() else {
            return;
        };
        let UndoState::Available(undo_id) = info.undo.clone() else {
            return;
        };
        match UndoRun::start(search, &record.place, &record.root, undo_id) {
            Ok(run) => {
                info.undo = UndoState::Running;
                self.replace.undos.push(run);
            }
            Err(error) => info.undo = UndoState::Failed(error),
        }
        self.results.touch();
    }

    /// The panel's replacement commands; `false` for every other action.
    pub(in crate::app) fn apply_replace_action(&mut self, action: &PanelAction) -> bool {
        let text = self.dialog.form.replacement.clone();
        let job = match action {
            PanelAction::Replace(rows) => self.results.replace_selected(rows, &text),
            PanelAction::ReplaceRemaining(search) => self.results.replace_remaining(*search, &text),
            PanelAction::Undo(search) => {
                self.undo_replace(*search);
                return true;
            }
            _ => return false,
        };
        let started = job.and_then(|job| self.ask_replace(job));
        self.panel.notice = started.err();
        true
    }

    /// Draws the confirmation and acts on its answer.
    pub(in crate::app) fn ui_replace_confirm(&mut self, ctx: &egui::Context) {
        let Some(confirm) = &mut self.replace.confirm else {
            return;
        };
        let escape =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let action = if escape {
            ConfirmAction::Cancel
        } else {
            confirm.show(ctx)
        };
        match action {
            ConfirmAction::None => {}
            ConfirmAction::Preview(text) => self.start_preview(text),
            ConfirmAction::Apply => self.apply_replace(),
            ConfirmAction::Cancel => {
                self.replace.close_confirm();
                if self.owner == KeyOwner::Dialog && !self.dialog.open {
                    self.owner = KeyOwner::Explorer;
                }
            }
        }
    }

    /// Moves the updates of the dry run, the runs and the restores into the
    /// confirmation and the records.
    pub(in crate::app) fn pump_replace(&mut self) {
        self.pump_preview();
        let mut runs = std::mem::take(&mut self.replace.runs);
        runs.retain_mut(|run| !self.pump_run(run));
        self.replace.runs = runs;
        let mut undos = std::mem::take(&mut self.replace.undos);
        undos.retain(|undo| {
            let Some(result) = undo.poll() else {
                return true;
            };
            if let Some(record) = self.results.record_mut(undo.search) {
                let state =
                    match result {
                        Ok(left) if left.is_empty() => UndoState::Done,
                        Ok(left) => {
                            let count = left.len() as u64;
                            // The files „Rückgängig“ left join the skipped rows.
                            record.skipped.extend(left.into_iter().map(|(rel, reason)| {
                                (rel, format!("nicht zurückgesetzt: {reason}"))
                            }));
                            UndoState::Partly(count)
                        }
                        Err(error) => UndoState::Failed(error),
                    };
                if let Some(info) = record.replace.as_mut() {
                    info.undo = state;
                }
            }
            self.results.touch();
            false
        });
        self.replace.undos = undos;
    }

    fn pump_preview(&mut self) {
        let Some(run) = &mut self.replace.preview else {
            return;
        };
        let (updates, end) = run.poll();
        let found = &mut self.replace.preview_found;
        for update in updates {
            match update {
                ReplaceUpdate::File { replacements, .. } => {
                    found.files_changed += 1;
                    found.replacements += replacements;
                }
                ReplaceUpdate::Skipped { rel, reason } => found.skipped.push((rel, reason)),
                ReplaceUpdate::Progress { files } => found.files_searched = files,
            }
        }
        let cancelled = run.cancelled();
        let Some(confirm) = &mut self.replace.confirm else {
            self.replace.preview = None;
            return;
        };
        confirm.state = match end {
            None => PreviewState::Running {
                files: found.files_changed,
                replacements: found.replacements,
            },
            Some(Ok(totals)) if totals.canceled => {
                PreviewState::Failed("Die Vorschau wurde abgebrochen.".into())
            }
            Some(Ok(totals)) => PreviewState::Ready(PreviewTotals {
                files_searched: totals.files_searched,
                files_changed: totals.files_changed,
                replacements: totals.replacements,
                skipped: std::mem::take(&mut found.skipped),
            }),
            Some(Err(_)) if cancelled => {
                PreviewState::Failed("Die Vorschau wurde abgebrochen.".into())
            }
            Some(Err(error)) => PreviewState::Failed(error),
        };
        if !matches!(confirm.state, PreviewState::Running { .. }) {
            self.replace.preview = None;
        }
    }

    /// One writing run; `true` once it ended and its record is final.
    fn pump_run(&mut self, run: &mut ReplaceRun) -> bool {
        let (updates, end) = run.poll();
        let rows_change = !updates.is_empty();
        let cancelled = run.cancelled();
        let Some(record) = self.results.record_mut(run.id) else {
            return end.is_some();
        };
        for update in updates {
            apply_update(record, update);
        }
        let Some(end) = end else {
            if rows_change {
                self.results.touch();
            }
            return false;
        };
        let changed: Vec<String> = record.files.iter().map(|file| file.rel.clone()).collect();
        let source = record.replace.as_ref().and_then(|info| info.source);
        match end {
            Ok(totals) => {
                if let Some(info) = record.replace.as_mut() {
                    info.replacements = totals.replacements;
                    info.files_searched = totals.files_searched;
                    info.undo = totals
                        .undo_id
                        .clone()
                        .map_or(UndoState::Unavailable, UndoState::Available);
                }
                record.progress.0 = totals.files_searched;
                record.state = if totals.canceled {
                    RecordState::Cancelled
                } else {
                    RecordState::Done(summary_of(&totals))
                };
            }
            Err(_) if cancelled => record.state = RecordState::Cancelled,
            Err(error) => record.state = RecordState::Failed(error),
        }
        // As in VS Code, replaced files leave the search they came from.
        if let Some(source) = source.and_then(|id| self.results.record_mut(id)) {
            source.files.retain(|file| !changed.contains(&file.rel));
            source.reindex();
        }
        self.results.touch();
        true
    }

    /// The line of a running replacement for the dialog.
    pub(in crate::app) fn replace_status(&self) -> Option<String> {
        let run = self.replace.runs.last()?;
        let record = self.results.record(run.id)?;
        let info = record.replace.as_ref()?;
        Some(format!(
            "Ersetzen läuft: {} durchsucht, {} Ersetzungen in {} Dateien",
            crate::app::content_search_results::group_thousands(record.progress.0),
            crate::app::content_search_results::group_thousands(info.replacements),
            crate::app::content_search_results::group_thousands(record.files.len() as u64)
        ))
    }
}

impl App {
    /// Strg+Umschalt+H: the dialog with „Ersetzen durch“ focused.
    pub(in crate::app) fn open_replace_in_files(&mut self, folder: Option<String>) {
        let folder = folder.unwrap_or_else(|| self.root_path.clone());
        let place = self.active_text_place();
        self.text_tools.dialog.open_for_replace(folder, place);
        self.text_tools.owner = KeyOwner::Dialog;
    }
}

fn apply_update(record: &mut SearchRecord, update: ReplaceUpdate) {
    match update {
        ReplaceUpdate::File { rel, replacements } => {
            if let Some(info) = record.replace.as_mut() {
                info.replacements += replacements;
            }
            match record.file_slot(&rel) {
                Some(slot) => record.files[slot].hits += replacements,
                None => record.push_file(FileHits {
                    rel,
                    size: 0,
                    mtime_ms: 0,
                    lines: Vec::new(),
                    hits: replacements,
                    expanded: false,
                }),
            }
        }
        ReplaceUpdate::Skipped { rel, reason } => record.skipped.push((rel, reason)),
        ReplaceUpdate::Progress { files } => {
            record.progress.0 = files;
            if let Some(info) = record.replace.as_mut() {
                info.files_searched = files;
            }
        }
    }
}
