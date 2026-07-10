use super::prelude::*;
use super::*;
use crate::app::delete_worker::DeleteReporter;
use std::sync::atomic::{AtomicBool, Ordering};

impl App {
    pub(super) fn ui_temp_recovery(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.label(
            RichText::new("REMOTE-WIEDERHERSTELLUNG")
                .small()
                .color(Color32::from_gray(140)),
        );
        let count = match recovery_session_count() {
            Ok(count) => count,
            Err(error) => {
                ui.colored_label(
                    Color32::from_rgb(230, 120, 100),
                    format!("Wiederherstellungsdaten konnten nicht geprüft werden: {error}"),
                );
                return;
            }
        };
        if count == 0 {
            ui.colored_label(Color32::from_gray(140), "Keine erhaltenen Sitzungen.");
            return;
        }
        ui.colored_label(
            Color32::from_rgb(255, 190, 90),
            format!(
                "{count} Sitzung(en) mit lokalen Remote-Datei-Kopien unter {}.",
                temp_root().display()
            ),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("Ordner öffnen").clicked() {
                open_local_path(&temp_root().to_string_lossy(), OpenMode::Default);
            }
            let can_clean = self.trash_rx.is_none() && self.trash_worker.is_none();
            if ui
                .add_enabled(
                    can_clean,
                    egui::Button::new("Wiederherstellungsdaten löschen…"),
                )
                .on_hover_text(
                    "Löscht nur erhaltene lokale Temp-Kopien; Remote-Dateien bleiben unverändert.",
                )
                .clicked()
                && confirm_yes_no(
                    "Wiederherstellungsdaten löschen",
                    &format!(
                        "{count} erhaltene Sitzung(en) endgültig löschen? Die Remote-Dateien werden nicht verändert."
                    ),
                )
            {
                self.start_temp_recovery_cleanup();
            }
        });
    }

    fn start_temp_recovery_cleanup(&mut self) {
        let plan = match recovery_delete_plan() {
            Ok(plan) => plan,
            Err(error) => {
                self.error_msg = Some(format!(
                    "Wiederherstellungsdaten konnten nicht geplant werden: {error}"
                ));
                return;
            }
        };
        if plan.directories.is_empty() {
            self.error_msg =
                Some("Keine sicher löschbaren Wiederherstellungssitzungen gefunden.".to_string());
            return;
        }
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let attempted = plan.discovered;
        let initial = DeleteProgress::new(DeleteKind::Permanent, DeleteOrigin::Recovery, attempted);
        let worker_progress = initial.clone();
        let spawn = std::thread::Builder::new()
            .name("temp-recovery-cleanup".into())
            .spawn(move || {
                let retained = plan.directories.len();
                let mut outcome =
                    DeleteOutcome::new(DeleteKind::Permanent, DeleteOrigin::Recovery, attempted);
                let mut reporter = DeleteReporter::new(tx, worker_cancel.clone(), worker_progress);
                for directory in plan.directories {
                    let display = directory.to_string_lossy().replace('\\', "/");
                    if worker_cancel.load(Ordering::Acquire)
                        || !reporter.begin_target(&display, DeletePhase::Planning)
                    {
                        outcome.canceled = true;
                        break;
                    }
                    let result = remove_recovery_session_controlled(
                        &directory,
                        &worker_cancel,
                        |progress| reporter.recursive_progress(progress),
                    );
                    match result {
                        Ok(report)
                            if report.status == crate::vfs::RecursiveDeleteStatus::Complete =>
                        {
                            outcome.entries_planned =
                                outcome.entries_planned.saturating_add(report.planned);
                            outcome.entries_deleted =
                                outcome.entries_deleted.saturating_add(report.removed);
                            outcome.record_success(display);
                            reporter.finish_target(true, false);
                        }
                        Ok(report) => {
                            outcome.entries_planned =
                                outcome.entries_planned.saturating_add(report.planned);
                            outcome.entries_deleted =
                                outcome.entries_deleted.saturating_add(report.removed);
                            outcome.processed = outcome.processed.saturating_add(1);
                            outcome.partial_mutation |= report.removed > 0;
                            outcome.canceled = true;
                            reporter.finish_target(false, false);
                            break;
                        }
                        Err(failure) => {
                            outcome.entries_planned =
                                outcome.entries_planned.saturating_add(failure.planned);
                            outcome.entries_deleted =
                                outcome.entries_deleted.saturating_add(failure.removed);
                            outcome.partial_mutation |= failure.removed > 0;
                            outcome.record_error(display, failure.error.to_string());
                            reporter.finish_target(false, false);
                        }
                    }
                }
                if worker_cancel.load(Ordering::Acquire) && outcome.processed < outcome.attempted {
                    outcome.canceled = true;
                }
                if retained < attempted && !outcome.canceled {
                    outcome.record_aux_error(
                        "Wiederherstellungslimit".to_string(),
                        format!(
                            "{} Sitzungen blieben wegen des Limits unverarbeitet",
                            attempted - retained
                        ),
                    );
                }
                reporter.finish(outcome);
            });
        self.install_delete_worker(spawn, rx, cancel, initial, DeleteOrigin::Recovery);
    }
}
