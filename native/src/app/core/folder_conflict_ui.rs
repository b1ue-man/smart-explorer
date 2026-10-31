//! Folder admission shared by Explorer and the se Desktop surface.
use crate::transfer::{FolderConflict, JobItems, Layout, TransferJob};
use eframe::egui;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// The window's context once the admission was shown: the preview thread
/// wakes the GUI through it when its result is ready (M12).
type RepaintSlot = Arc<Mutex<Option<egui::Context>>>;

pub(crate) struct Admission {
    job: Option<TransferJob>,
    preview: crate::transfer::FolderPreviewScan,
    waker: RepaintSlot,
    index: usize,
    choice: FolderConflict,
    all: bool,
}

impl Admission {
    pub(crate) fn new(job: TransferJob) -> Result<Self, String> {
        let waker = RepaintSlot::default();
        let wake = waker.clone();
        let preview = crate::transfer::FolderPreviewScan::start_with_wake(
            job.clone(),
            Box::new(move || {
                if let Some(ctx) = wake.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
                    ctx.request_repaint();
                }
            }),
        )
        .map_err(|error| format!("Ordnerprüfung nicht gestartet: {error}"))?;
        Ok(Self {
            job: Some(job),
            preview,
            waker,
            index: 0,
            choice: FolderConflict::KeepBoth,
            all: false,
        })
    }

    /// `Some(Ok(None))` is an explicit cancellation, never approval.
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
    ) -> Option<Result<Option<TransferJob>, String>> {
        self.waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(|| ctx.clone());
        let state = self.preview.snapshot();
        if let Some(error) = &state.error {
            return Some(Err(error.clone()));
        }
        if state.ready && state.folders.is_empty() {
            return Some(Ok(self.job.take()));
        }
        let folder = state.folders.get(self.index).cloned();
        let count = state.folders.len();
        let remaining_support_keep_both = state
            .folders
            .iter()
            .skip(self.index)
            .all(|folder| folder.keep_both_supported);
        drop(state);
        let mut approved = false;
        let mut canceled = false;
        egui::Window::new("Zielordner entscheiden")
            .id(egui::Id::new("folder_conflict_admission"))
            .collapsible(false).resizable(true).default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if let Some(folder) = &folder {
                    ui.label(format!("Ordner {} von {count}: {}", self.index + 1, folder.relative));
                    if let Some(job) = &self.job { ui.label(format!("Quelle: {} · {}", job.source_label, folder.source)); }
                    if let Some(job) = &self.job { ui.label(format!("Ziel: {}", job.target_label)); }
                    ui.separator();
                    ui.add_enabled_ui(folder.keep_both_supported, |ui| { ui.radio_value(&mut self.choice, FolderConflict::KeepBoth, "Beide behalten"); });
                    if folder.keep_both_supported {
                        ui.label(format!("Vorgeschlagener Zielname: {} (wird beim Start exklusiv geprüft)", folder.keep_both));
                    } else {
                        ui.label("Diese Verbindung kann einen freien Ordnernamen bei gleichzeitigen Kopien nicht zusichern. Zusammenführen oder Überspringen ausdrücklich wählen.");
                    }
                    ui.radio_value(&mut self.choice, FolderConflict::Merge, "Zusammenführen");
                    if let Some(job) = &self.job {
                        ui.label(format!(
                            "Gleichnamige Dateien beim Zusammenführen: {}",
                            merge_rule(job)
                        ));
                    }
                    ui.radio_value(&mut self.choice, FolderConflict::Skip, "Überspringen");
                    ui.checkbox(&mut self.all, "Für alle verbleibenden Ordner übernehmen");
                    if self.all && self.choice == FolderConflict::KeepBoth && !remaining_support_keep_both {
                        ui.label("Mindestens ein verbleibender Ordner verlangt ausdrücklich Zusammenführen oder Überspringen.");
                    }
                    ui.separator();
                    if let Some(job) = &self.job {
                        ui.label(count_text(job, folder));
                    }
                    for sample in &folder.samples { ui.label(sample); }
                    if !folder.complete {
                        ui.label(format!("{}; {} geschützte oder nicht lesbare Bereiche", if folder.counted { "Zählung unvollständig" } else { "Zählung läuft" }, folder.omitted));
                    }
                    if !folder.omitted_paths.is_empty() {
                        ui.label(format!("Angezeigte Auslassungen: {} von {}", folder.omitted_paths.len(), folder.omitted));
                        for path in &folder.omitted_paths { ui.label(path); }
                    }
                    if let Some(note) = &folder.note { ui.label(note); }
                    ui.horizontal(|ui| {
                        let allowed = self.choice != FolderConflict::KeepBoth || folder.keep_both_supported && (!self.all || remaining_support_keep_both);
                        approved = ui.add_enabled(allowed, egui::Button::new("Übernehmen")).clicked();
                        canceled = ui.button("Abbrechen").clicked();
                    });
                } else {
                    ui.spinner(); ui.label("Oberste Quellordner und Zielnamen werden geprüft…");
                    canceled = ui.button("Abbrechen").clicked();
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        if canceled {
            self.preview.cancel();
            return Some(Ok(None));
        }
        if approved {
            let state = self.preview.snapshot();
            if let Some(error) = state.error {
                return Some(Err(error));
            }
            if self.index >= state.folders.len() {
                return None;
            }
            if let Some(job) = &mut self.job {
                let end = if self.all {
                    state.folders.len()
                } else {
                    self.index + 1
                };
                for folder in &state.folders[self.index..end] {
                    job.folders.roots.insert(folder.source.clone(), self.choice);
                }
                self.index = end;
                if self.index == state.folders.len() {
                    self.preview.cancel();
                    if matches!(&job.items, JobItems::Roots { paths, .. } if paths.iter().all(|path| job.folders.roots.get(path) == Some(&FolderConflict::Skip)))
                    {
                        return Some(Ok(None));
                    }
                    return Some(Ok(self.job.take()));
                }
                self.choice = FolderConflict::KeepBoth;
            }
        }
        None
    }
}

/// What happens to a same-named file of a merged folder, as the dialog says it.
fn merge_rule(job: &TransferJob) -> &'static str {
    use crate::types::Conflict;
    if job.folders.merge_update {
        return "neuere ersetzen ältere, Gleiche werden übersprungen, ein neueres Ziel bleibt, bei gleicher Zeit und anderer Größe bleiben beide";
    }
    match job.conflict {
        Conflict::Rename => "beide behalten",
        Conflict::Overwrite => "überschreiben",
        Conflict::Skip => "überspringen",
        Conflict::Update => "neuere ersetzen ältere, Gleiche werden übersprungen, ein neueres Ziel bleibt, bei gleicher Zeit und anderer Größe bleiben beide",
    }
}

/// The counts of the preview of one folder, as one line. The five counts of
/// the file rule are shown only for a job that applies it
/// (`TransferJob::applies_update_rule`); else the folder shows what the job's
/// own file policy acts on: new and same-named files, as before the rule.
fn count_text(job: &TransferJob, folder: &crate::transfer::FolderConflictPreview) -> String {
    if !job.applies_update_rule() {
        return format!(
            "Zusammenführen: bisher {} neue und {} gleichnamige Dateien",
            folder.new_files, folder.colliding_files
        );
    }
    format!(
        "Zusammenführen: bisher {} neu, {} neuer (ersetzen), {} gleich (übersprungen), {} Ziel neuer (bleibt), {} Konflikt (beide behalten)",
        folder.new_files,
        folder.newer_files,
        folder.equal_files,
        folder.target_newer_files,
        folder.conflict_files
    )
}

pub(in crate::app) fn requires_preview(job: &TransferJob) -> bool {
    job.resume.is_none()
        && job.layout == Layout::Tree
        && matches!(job.items, JobItems::Roots { .. })
}

type Queue = VecDeque<(Admission, super::transfer_center::TransferPurpose)>;
fn queue() -> &'static Mutex<Queue> {
    static QUEUE: OnceLock<Mutex<Queue>> = OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub(in crate::app) fn enqueue(
    job: TransferJob,
    purpose: super::transfer_center::TransferPurpose,
) -> Result<(), String> {
    let admission = Admission::new(job)?;
    queue()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push_back((admission, purpose));
    Ok(())
}

impl super::App {
    pub(in crate::app) fn ui_folder_conflicts(&mut self, ctx: &egui::Context) {
        let finished = {
            let mut queue = queue()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let result = queue
                .front_mut()
                .and_then(|(admission, _)| admission.show(ctx));
            result.and_then(|result| queue.pop_front().map(|(_, purpose)| (result, purpose)))
        };
        if let Some((result, purpose)) = finished {
            match result {
                Ok(Some(job)) => {
                    self.submit_approved_job_for(job, purpose);
                }
                Ok(None) => {}
                Err(error) => self.error_msg = Some(format!("Ordnerkonflikte prüfen: {error}")),
            }
        }
    }
}
