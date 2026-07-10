use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn drain_trash(&mut self) {
        let Some(rx) = self.trash_rx.take() else {
            return;
        };
        let mut outcome = None;
        let mut disconnected = false;
        for _ in 0..32 {
            match rx.try_recv() {
                Ok(DeleteMsg::Progress(progress)) => self.trash_progress = Some(progress),
                Ok(DeleteMsg::Finished(finished)) => {
                    outcome = Some(finished);
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if let Some(outcome) = outcome {
            let expected_origin = self.trash_origin;
            let join_error = self.finish_delete_worker();
            self.apply_delete_successes(&outcome);
            self.report_delete_outcome(&outcome, join_error, expected_origin);
        } else if disconnected {
            let origin = self.trash_origin;
            let progress = self.trash_progress.clone();
            let join_error = self.finish_delete_worker();
            let detail = join_error.unwrap_or_else(|| match progress {
                Some(ref progress) => format!(
                    "Letzter Stand: {} von {} Zielen verarbeitet, {} Einträge gelöscht",
                    progress.targets_processed, progress.targets_total, progress.entries_deleted
                ),
                None => "Worker-Kanal ohne Abschlussmeldung geschlossen".to_string(),
            });
            self.error_msg = Some(format!(
                "Löschvorgang ohne bestätigtes Ergebnis beendet; der Zustand kann teilweise geändert sein. {detail}"
            ));
            self.mark_unconfirmed_delete_state(origin);
        } else {
            self.trash_rx = Some(rx);
        }
    }

    fn finish_delete_worker(&mut self) -> Option<String> {
        self.trash_rx = None;
        self.trash_cancel = None;
        self.trash_progress = None;
        self.trash_origin = None;
        self.trash_worker
            .take()
            .and_then(|worker| match worker.join() {
                Ok(()) => None,
                Err(payload) => Some(format!(
                    "Lösch-Worker-Panik: {}",
                    worker_panic_detail(payload)
                )),
            })
    }

    fn report_delete_outcome(
        &mut self,
        outcome: &DeleteOutcome,
        join_error: Option<String>,
        expected_origin: Option<DeleteOrigin>,
    ) {
        let origin_mismatch = expected_origin != Some(outcome.origin);
        if outcome.partial_mutation {
            self.mark_unconfirmed_delete_state(Some(outcome.origin));
        }
        if let Some(error) = join_error {
            self.error_msg = Some(error);
            return;
        }
        if origin_mismatch {
            self.error_msg = Some("Lösch-Worker meldete einen unerwarteten Ursprung.".to_string());
            return;
        }
        if outcome.canceled {
            self.notice = Some((
                format!(
                    "⚠ Löschen abgebrochen: {} von {} Zielen bestätigt abgeschlossen",
                    outcome.succeeded, outcome.attempted
                ),
                std::time::Instant::now(),
            ));
        }
        if !outcome.errors.is_empty() || outcome.suppressed_errors > 0 {
            let failed = outcome.processed.saturating_sub(outcome.succeeded);
            let unprocessed = outcome.attempted.saturating_sub(outcome.processed);
            let first = outcome
                .errors
                .first()
                .map(|(path, detail)| format!("{path}: {detail}"))
                .unwrap_or_else(|| "Weitere Fehler unterdrückt".to_string());
            self.error_msg = Some(format!(
                "{}: {} erfolgreich, {} fehlgeschlagen, {} nicht verarbeitet. {}",
                outcome.kind.error_context(),
                outcome.succeeded,
                failed,
                unprocessed,
                first
            ));
        } else if !outcome.canceled {
            self.notice = Some((
                outcome.kind.success_text(outcome.succeeded),
                std::time::Instant::now(),
            ));
        }
    }

    fn apply_delete_successes(&mut self, outcome: &DeleteOutcome) {
        match outcome.origin {
            DeleteOrigin::Reclaim => {
                if let Some(report) = &mut self.reclaim_report {
                    report.prune_paths(&outcome.succeeded_paths);
                }
                self.reclaim_selected
                    .retain(|path| !outcome.succeeded_paths.iter().any(|done| done == path));
            }
            DeleteOrigin::Explorer => {
                self.entries.retain(|entry| {
                    !outcome.succeeded_paths.iter().any(|done| {
                        entry.path.as_ref() == done
                            || entry
                                .path
                                .strip_prefix(done.as_str())
                                .is_some_and(|rest| rest.starts_with('/'))
                    })
                });
                let remaining: HashSet<Arc<str>> =
                    self.entries.iter().map(FileEntry::key).collect();
                self.selection.retain(|key| remaining.contains(key));
                if self
                    .cursor
                    .as_ref()
                    .is_some_and(|key| !remaining.contains(key))
                {
                    self.cursor = None;
                }
                self.recompute_view();
            }
            DeleteOrigin::Recovery => {}
        }
    }

    fn mark_unconfirmed_delete_state(&mut self, origin: Option<DeleteOrigin>) {
        match origin {
            Some(DeleteOrigin::Explorer) => self.rescan(),
            Some(DeleteOrigin::Reclaim) => {
                self.reclaim_state = StorageRunState::Partial;
                self.reclaim_issues.push(
                    "Ein Löschvorgang kann einen ausgewählten Baum nur teilweise geändert haben; bitte Reclaim neu scannen."
                        .to_string(),
                );
            }
            Some(DeleteOrigin::Recovery) => {}
            None => {}
        }
    }

    pub(in crate::app) fn shutdown_delete_worker(&mut self) {
        let active = self.trash_worker.is_some() || self.trash_rx.is_some();
        if !active {
            return;
        }
        if let Some(cancel) = &self.trash_cancel {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        let rx = self.trash_rx.take();
        let join_result = self.trash_worker.take().map(|worker| worker.join());
        let mut final_outcome = None;
        if let Some(rx) = rx {
            for message in rx.try_iter() {
                match message {
                    DeleteMsg::Progress(progress) => self.trash_progress = Some(progress),
                    DeleteMsg::Finished(outcome) => final_outcome = Some(outcome),
                }
            }
        }
        match (join_result, final_outcome) {
            (Some(Err(payload)), _) => eprintln!(
                "Lösch-Worker beim Beenden fehlgeschlagen: {}",
                worker_panic_detail(payload)
            ),
            (_, Some(outcome)) if outcome.canceled || outcome.partial_mutation => eprintln!(
                "Löschvorgang beim Beenden abgebrochen: {} von {} Zielen abgeschlossen, {} Einträge gelöscht",
                outcome.succeeded, outcome.attempted, outcome.entries_deleted
            ),
            (_, Some(outcome)) if !outcome.errors.is_empty() || outcome.suppressed_errors > 0 => {
                eprintln!(
                    "Löschvorgang beim Beenden teilweise fehlgeschlagen: {} erfolgreich, {} Fehler",
                    outcome.succeeded,
                    outcome.errors.len() as u64 + outcome.suppressed_errors
                );
            }
            (_, Some(_)) => {}
            _ => eprintln!(
                "Löschvorgang beim Beenden ohne bestätigtes Abschlussresultat beendet"
            ),
        }
        self.trash_cancel = None;
        self.trash_progress = None;
        self.trash_origin = None;
    }
}

fn worker_panic_detail(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unbekannte Panik".to_string()
    }
}
