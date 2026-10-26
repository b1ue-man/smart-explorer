use super::prelude::*;
use super::*;

#[derive(Default)]
pub(super) struct ReadAccess {
    pending: Option<(String, Receiver<Result<bool, String>>)>,
    message: Option<String>,
    pub(super) denied: crate::scanner::DeniedRoots,
    generation: u64,
    consent_generation: u64,
    unretryable_errors: u64,
    initial_error: Option<String>,
    partial: Option<super::partial_scan::PartialWorker<super::partial_scan::ExplorerResult>>,
}

impl ReadAccess {
    pub(super) fn invalidate_partial(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.partial.take().is_some() {
            self.message = Some("Teilscans wegen geänderter Ansicht oder Dateisystemgeneration beendet; verbleibende Sperren bleiben anforderbar.".into());
        }
    }
    pub(super) fn new_scan(&mut self) {
        self.invalidate_partial();
        self.denied = Default::default();
        self.message = None;
    }
}

#[cfg(test)]
#[test]
fn search_recursive_access_task_declined_or_failed_consent_keeps_current_results() {
    let mut app = App::new_for_copy_task();
    app.root_path = "C:/task".into();
    for result in [Ok(false), Err("helper failed".to_string())] {
        let (tx, rx) = crossbeam_channel::bounded(1);
        app.read_access.pending = Some((app.root_path.clone(), rx));
        app.selection.insert(Arc::from("retained-selection"));
        tx.send(result).unwrap();
        app.poll_read_access();
        assert!(app.selection.contains("retained-selection"));
        assert!(!app.scan_running);
        assert!(app.read_access.pending.is_none());
        assert!(app.read_access.message.is_some());
    }
}

impl App {
    pub(super) fn poll_read_access(&mut self) {
        self.poll_partial_read();
        let Some((root, receiver)) = &self.read_access.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Err("Lesehelfer wurde ohne Ergebnis beendet".into())
            }
        };
        let same_location = self.remote.is_none()
            && self.root_path == *root
            && self.read_access.generation == self.read_access.consent_generation
            && !self.disconnected_drive(root)
            && !self.scan_running;
        self.read_access.pending = None;
        self.read_access.message = Some(match result {
            Ok(true) => {
                if same_location {
                    match self.read_access.denied.roots() {
                        Ok(roots) => {
                            self.read_access.initial_error = self.error_msg.clone();
                            self.read_access.unretryable_errors = self
                                .progress
                                .errors
                                .saturating_sub(self.progress.permission_denied);
                            let scope = self.scan_scope();
                            match super::partial_scan::explorer(
                                self.root_path.clone(),
                                self.read_access.generation,
                                roots,
                                scope.max_depth,
                                scope.retention,
                            ) {
                                Ok(worker) => self.read_access.partial = Some(worker),
                                Err(error) => self.error_msg = Some(error),
                            }
                        }
                        Err(error) => self.error_msg = Some(format!("Wiederleseindex: {error}")),
                    }
                }
                if self.read_access.partial.is_some() {
                    "Leserechte verfügbar. Gesperrte Teilbäume werden aktualisiert.".into()
                } else {
                    "Leserechte verfügbar. Die Ansicht wurde inzwischen geändert oder der Wiederleseweg konnte nicht starten; bisherige Ergebnisse bleiben erhalten.".into()
                }
            }
            Ok(false) => {
                "Rechteanfrage abgebrochen. Die bisherigen Ergebnisse bleiben erhalten.".into()
            }
            Err(error) => format!(
                "Lesezugriff nicht erweitert: {error}. Die bisherigen Ergebnisse bleiben erhalten."
            ),
        });
    }

    fn poll_partial_read(&mut self) {
        let Some(worker) = &self.read_access.partial else {
            return;
        };
        if worker.generation != self.read_access.generation
            || worker.root != self.root_path
            || self.remote.is_some()
            || self.scan_running
            || self.disconnected_drive(&worker.root)
        {
            self.read_access.partial = None;
            return;
        }
        let message = worker.rx.try_recv();
        match message {
            Ok((root, result)) => {
                let old_visits = match self.read_access.denied.visits(&root) {
                    Ok(Some(visits)) if result.measured => visits,
                    Ok(_) => {
                        self.error_msg = Some(format!("Teilbaum {root} nicht übernommen: Besuchssumme fehlt. Vollständiges Aktualisieren stellt die Zählgrundlage wieder her."));
                        return;
                    }
                    Err(error) => {
                        self.error_msg = Some(error.to_string());
                        return;
                    }
                };
                if !result.complete && old_visits != crate::scanner::ScanVisits::default() {
                    // An incomplete walk and a filtered old listing cannot
                    // prove their exact union. Keep the old data AND counters.
                    self.error_msg = Some(format!("Teilbaum {root} bleibt unvollständig. Bisherige Daten und Besuchszähler bleiben bis zu einer vollständigen Wiederlesung erhalten."));
                    return;
                }
                let visits = match result.denied.visits(&root) {
                    Ok(Some(visits))
                        if visits.scanned == result.progress.scanned
                            && visits.bytes == result.progress.bytes =>
                    {
                        visits
                    }
                    Ok(_) => {
                        self.error_msg = Some(format!("Teilbaum {root} nicht übernommen: neue Besuchssumme ist nicht gesichert."));
                        return;
                    }
                    Err(error) => {
                        self.error_msg = Some(error.to_string());
                        return;
                    }
                };
                let total = self
                    .progress
                    .scanned
                    .checked_sub(old_visits.scanned)
                    .zip(self.progress.bytes.checked_sub(old_visits.bytes))
                    .and_then(|(scanned, bytes)| {
                        crate::scanner::ScanVisits { scanned, bytes }.checked_add(visits)
                    });
                let Some(total) = total else {
                    self.error_msg = Some(format!("Teilbaum {root} nicht übernommen: Besuchssummen sind nicht konsistent. Vollständiges Aktualisieren ist erforderlich."));
                    return;
                };
                let remaining = match self.read_access.denied.roots() {
                    Ok(roots) => roots,
                    Err(error) => {
                        self.error_msg = Some(error.to_string());
                        self.read_access.partial = None;
                        return;
                    }
                };
                let next = crate::scanner::DeniedRoots::default();
                for path in remaining {
                    if path != root || !result.complete {
                        next.record(path);
                    }
                }
                match result.denied.roots() {
                    Ok(roots) => {
                        for path in roots {
                            next.record(path);
                        }
                    }
                    Err(error) => {
                        self.error_msg = Some(error.to_string());
                        next.record(root.clone());
                    }
                }
                if let Err(error) = self.read_access.denied.copy_visits_to(&next) {
                    self.error_msg = Some(error.to_string());
                    return;
                }
                if !result.complete {
                    next.record_visits(root.clone(), visits);
                }
                self.read_access.denied = next;
                super::partial_scan::merge_entries(
                    &mut self.entries,
                    &root,
                    result.entries,
                    result.complete,
                );
                if result.complete {
                    self.failed_paths
                        .retain(|(path, _)| !crate::scanner::retry_contains(&root, path));
                }
                self.progress.permission_denied = self
                    .read_access
                    .denied
                    .roots()
                    .map(|roots| roots.len() as u64)
                    .unwrap_or(1);
                self.progress.scanned = total.scanned;
                self.progress.bytes = total.bytes;
                self.read_access.unretryable_errors =
                    self.read_access.unretryable_errors.saturating_add(
                        result
                            .progress
                            .errors
                            .saturating_sub(result.progress.permission_denied),
                    );
                self.progress.errors = self
                    .read_access
                    .unretryable_errors
                    .saturating_add(self.progress.permission_denied);
                self.summary_cache = None;
                self.recompute_view();
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.read_access.partial = None;
                if self.progress.permission_denied == 0
                    && self.read_access.unretryable_errors == 0
                    && self.error_msg == self.read_access.initial_error
                {
                    self.error_msg = None;
                }
                self.read_access.message = Some("Teilscans beendet; nicht übernommene oder weiterhin gesperrte Wurzeln bleiben erneut anforderbar.".into());
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
    }

    pub(super) fn ui_read_access(&mut self, ui: &mut egui::Ui) {
        if self.progress.permission_denied > 0 {
            ui.label(format!(
                "{} Zugriff(e) verweigert",
                self.progress.permission_denied
            ));
            if self.remote.is_none() && crate::local_access::can_request_access(&self.root_path) {
                if ui.add_enabled(self.read_access.pending.is_none() && self.read_access.partial.is_none() && !self.scan_running,
                    egui::Button::new("Leserechte anfordern …"))
                    .on_hover_text("Geschützte Dateien nach Windows-Zustimmung in diesem Fenster lesen; die Ordnerrechte bleiben unverändert.")
                    .clicked() {
                    self.read_access.consent_generation = self.read_access.generation;
                    let root = self.root_path.clone();
                    let worker_root = root.clone();
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    match std::thread::Builder::new().name("explorer-consent".into()).spawn(move || {
                        let _ = tx.send(crate::local_access::request_access(&worker_root));
                    }) {
                        Ok(_) => self.read_access.pending = Some((root, rx)),
                        Err(error) => self.read_access.message = Some(format!("Rechteanfrage konnte nicht starten: {error}")),
                    }
                }
            } else {
                ui.label("Verbleibende Sperren: Berechtigungen oder Anmeldung am Dateisystem/Anbieter prüfen.");
            }
        }
        if self.read_access.pending.is_some() || self.read_access.partial.is_some() {
            ui.spinner();
            ui.label(if self.read_access.partial.is_some() {
                "Gesperrte Teilbäume werden erneut gelesen …"
            } else {
                "Windows-Rechteanfrage läuft …"
            });
            if self.read_access.partial.is_some() && ui.button("Teilscans abbrechen").clicked() {
                self.read_access.invalidate_partial();
                self.read_access.message = Some("Teilscans abgebrochen; bisherige Ergebnisse und verbleibende Sperren bleiben erhalten.".into());
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
        }
        if let Some(message) = &self.read_access.message {
            ui.label(message);
        }
    }
}
