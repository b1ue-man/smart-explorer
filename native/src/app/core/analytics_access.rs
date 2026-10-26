use super::{App, StorageRunState, StorageScanSource};
use crate::analytics::ScanIssue;
use crate::app::theme;
use eframe::egui;

#[derive(Default)]
pub(super) struct AnalyticsAccess {
    pub permission_denied: u64,
    pub offer: bool,
    pub pending: Option<crossbeam_channel::Receiver<(String, Result<bool, String>)>>,
    pub message: Option<String>,
    pub(super) denied: crate::scanner::DeniedRoots,
    generation: u64,
    consent_generation: u64,
    partial: Option<super::partial_scan::PartialWorker<crate::analytics::ScanOutcome>>,
    prior_incomplete: bool,
}

impl AnalyticsAccess {
    pub(super) fn invalidate_partial(&mut self) -> bool {
        self.generation = self.generation.wrapping_add(1);
        let running = self.partial.take().is_some();
        if running {
            self.message = Some("Teilscans wegen Abbruch oder geänderter Dateisystemgeneration beendet; verbleibende Sperren bleiben anforderbar.".into());
        }
        running
    }

    pub(super) fn reset_scan(&mut self) {
        self.invalidate_partial();
        self.denied = Default::default();
        self.prior_incomplete = false;
        self.permission_denied = 0;
        self.offer = false;
        self.message = None;
    }
}

fn access_offer(
    denied: u64,
    source: Option<&StorageScanSource>,
    available: impl FnOnce(&str) -> bool,
) -> bool {
    if denied == 0 {
        return false;
    }
    match source {
        Some(StorageScanSource::Local { root }) => available(root),
        _ => false,
    }
}

pub(super) fn should_log_failure(state: StorageRunState, offer: bool) -> bool {
    state == StorageRunState::Failed && !offer
}

pub(super) fn issue_report(issues: &[ScanIssue], suppressed: u64, denied: u64) -> String {
    let mut report = format!(
        "{} Leseproblem(e), davon {denied} Zugriff verweigert.\n",
        (issues.len() as u64).saturating_add(suppressed)
    );
    for issue in issues {
        report.push_str(&format!("{}: {}\n", issue.path, issue.detail));
    }
    if suppressed > 0 {
        report.push_str(&format!(
            "{suppressed} weitere Probleme sind nicht einzeln gespeichert.\n"
        ));
    }
    report
}

pub(super) fn issues_ui(
    ui: &mut egui::Ui,
    issues: &[ScanIssue],
    suppressed: u64,
    denied: u64,
    notes: &[String],
) {
    for note in notes {
        ui.label(egui::RichText::new(note).small().color(theme::muted(ui)));
    }
    if issues.is_empty() && suppressed == 0 {
        return;
    }
    egui::CollapsingHeader::new("Leseprobleme – betroffene Pfade und Details")
        .id_salt("analytics_read_issues")
        .show(ui, |ui| {
            if ui.button("Bericht kopieren").clicked() {
                ui.ctx().copy_text(issue_report(issues, suppressed, denied));
            }
            egui::ScrollArea::vertical()
                .min_scrolled_height(0.0)
                .id_salt("analytics_read_issues_scroll")
                .max_height(super::dialog_geometry::remaining_height(ui, 2.0))
                .show(ui, |ui| {
                    for issue in issues {
                        ui.label(egui::RichText::new(&issue.path).strong());
                        ui.label(&issue.detail);
                        ui.separator();
                    }
                    if suppressed > 0 {
                        ui.label(format!(
                            "{suppressed} weitere Probleme nicht einzeln gespeichert."
                        ));
                    }
                });
        });
}

impl App {
    pub(super) fn log_analytics_outcome(&mut self) {
        if !should_log_failure(self.analytics_state, self.analytics_access.offer) {
            return;
        }
        let first = self
            .analytics_issues
            .first()
            .map(|issue| format!("{}: {}", issue.path, issue.detail))
            .unwrap_or_else(|| "Unbekannter Scan-Fehler".to_string());
        let count = self.analytics_issues.len() as u64 + self.analytics_suppressed_issues;
        self.push_app_error(
            "Speicher-Analyse",
            format!("{count} Leseproblem(e); erstes Problem: {first}"),
        );
    }

    pub(super) fn install_analytics_retry_index(
        &mut self,
        outcome: &crate::analytics::ScanOutcome,
    ) {
        self.analytics_access.denied = outcome.denied_roots.clone();
        self.analytics_access.prior_incomplete = (outcome.issues.len() as u64)
            .saturating_add(outcome.suppressed_issues)
            > outcome.permission_denied;
    }

    pub(super) fn update_analytics_access(&mut self, denied: u64) {
        self.analytics_access.permission_denied = denied;
        self.analytics_access.offer = access_offer(
            denied,
            self.analytics_source.as_ref(),
            crate::local_access::can_request_access,
        );
    }

    pub(super) fn request_analytics_access(&mut self) {
        if !self.analytics_access.offer
            || self.analytics_access.pending.is_some()
            || self.analytics_access.partial.is_some()
            || self.analytics_scan.is_some()
        {
            return;
        }
        let Some(StorageScanSource::Local { root }) = &self.analytics_source else {
            return;
        };
        self.analytics_access.consent_generation = self.analytics_access.generation;
        let root = root.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        match std::thread::Builder::new()
            .name("analytics-consent".into())
            .spawn(move || {
                let result = crate::local_access::request_access(&root);
                let _ = tx.send((root, result));
            }) {
            Ok(_) => {
                self.analytics_access.pending = Some(rx);
                self.analytics_access.message = None;
            }
            Err(error) => {
                self.analytics_access.message =
                    Some(format!("Rechteanfrage fehlgeschlagen: {error}"))
            }
        }
    }

    fn poll_partial_analytics(&mut self) {
        let Some(worker) = &self.analytics_access.partial else {
            return;
        };
        if worker.generation != self.analytics_access.generation
            || self.analytics_scan.is_some()
            || !matches!(&self.analytics_source, Some(StorageScanSource::Local { root }) if root == &worker.root)
        {
            self.analytics_access.partial = None;
            return;
        }
        if worker.cancel.load(std::sync::atomic::Ordering::Acquire) {
            self.analytics_access.partial = None;
            self.analytics_state = StorageRunState::Canceled;
            return;
        }
        let message = worker.rx.try_recv();
        match message {
            Ok((path, mut outcome)) => {
                let base = self.analytics_root().to_string();
                let previous = match self.analytics_access.denied.roots_with_sizes() {
                    Ok(roots) => roots,
                    Err(error) => {
                        self.analytics_issues.push(ScanIssue {
                            path,
                            detail: error.to_string(),
                        });
                        self.analytics_access.invalidate_partial();
                        self.analytics_state = StorageRunState::Partial;
                        return;
                    }
                };
                let previous_size = previous
                    .iter()
                    .find(|(root, _)| root == &path)
                    .map(|(_, size)| *size)
                    .unwrap_or(0);
                let measurements = match self.analytics_access.denied.measurements() {
                    Ok(measurements) => measurements,
                    Err(error) => {
                        self.analytics_issues.push(ScanIssue {
                            path,
                            detail: error.to_string(),
                        });
                        self.analytics_access.invalidate_partial();
                        self.analytics_state = StorageRunState::Partial;
                        return;
                    }
                };
                let complete = outcome.status == crate::analytics::ScanStatus::Complete;
                // A newly readable formerly empty subtree may still contain
                // independent locked children. Their roots replace its retry
                // root while the readable data can already be shown safely.
                let mergeable = complete
                    || (outcome.status == crate::analytics::ScanStatus::Partial
                        && (previous_size == 0
                            || self.analytics_tree.as_ref().is_some_and(|previous| {
                                super::partial_scan::prepare_partial_analytics(
                                    previous,
                                    &base,
                                    &path,
                                    &mut outcome,
                                )
                            })));
                let replacement_size = outcome
                    .tree
                    .as_ref()
                    .map_or(previous_size, |tree| tree.size);

                let merged = if mergeable {
                    let depth = super::partial_scan::relative_depth(&base, &path);
                    let parts: Vec<_> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
                    match (depth, self.analytics_tree.as_mut(), outcome.tree.take()) {
                        (Some(depth), Some(tree), Some(replacement)) => {
                            let start = parts.len().saturating_sub(depth as usize);
                            let mut ancestor = base.trim_end_matches('/').to_string();
                            let sizes: Option<Vec<u64>> = parts[start..]
                                .iter()
                                .map(|segment| {
                                    ancestor.push('/');
                                    ancestor.push_str(segment);
                                    measurements
                                        .iter()
                                        .find(|(path, _)| {
                                            crate::scanner::retry_contains(path, &ancestor)
                                                && crate::scanner::retry_contains(&ancestor, path)
                                        })
                                        .map(|(_, size)| *size)
                                })
                                .collect();
                            sizes.is_some_and(|sizes| {
                                crate::analytics::merge_retry_tree(
                                    tree,
                                    &parts[start..],
                                    replacement,
                                    &sizes,
                                )
                            })
                        }
                        (Some(0), None, Some(replacement)) => {
                            self.analytics_tree = Some(replacement);
                            true
                        }
                        _ => false,
                    }
                } else {
                    false
                };
                let next = crate::scanner::DeniedRoots::default();
                for (root, size) in previous {
                    if root != path || !merged {
                        next.record(root.clone());
                        next.record_size(root, size);
                    }
                }
                match outcome.denied_roots.roots_with_sizes() {
                    Ok(roots) => {
                        for (root, size) in roots {
                            if merged {
                                next.record(root.clone());
                                next.record_size(root, size);
                            }
                        }
                    }
                    Err(error) => {
                        next.record(path.clone());
                        next.record_size(path.clone(), previous_size);
                        self.analytics_issues.push(ScanIssue {
                            path: path.clone(),
                            detail: error.to_string(),
                        });
                    }
                }
                for (measured, size) in measurements {
                    let size = if merged && crate::scanner::retry_contains(&measured, &path) {
                        size.saturating_sub(previous_size)
                            .saturating_add(replacement_size)
                    } else {
                        size
                    };
                    next.record_size(measured, size);
                }
                if merged {
                    if let Err(error) = outcome.denied_roots.copy_sizes_to(&next) {
                        self.analytics_issues.push(ScanIssue {
                            path: path.clone(),
                            detail: error.to_string(),
                        });
                    }
                }
                self.analytics_access.denied = next;
                if merged {
                    self.analytics_issues
                        .retain(|issue| !crate::scanner::retry_contains(&path, &issue.path));
                }
                self.analytics_access.prior_incomplete |= outcome
                    .suppressed_issues
                    .saturating_add(outcome.issues.len() as u64)
                    > outcome.permission_denied;
                for issue in outcome.issues {
                    if self.analytics_issues.len() < 64 {
                        self.analytics_issues.push(issue);
                    } else {
                        self.analytics_suppressed_issues =
                            self.analytics_suppressed_issues.saturating_add(1);
                    }
                }
                let remaining_notes = 16usize.saturating_sub(self.analytics_notes.len());
                self.analytics_notes
                    .extend(outcome.notes.into_iter().take(remaining_notes));
                self.analytics_suppressed_issues = self
                    .analytics_suppressed_issues
                    .saturating_add(outcome.suppressed_issues);
                // Preserve exact completeness conservatively when old UI
                // diagnostics were suppressed; no partial walk seeds an index.
                self.analytics_access.permission_denied = self
                    .analytics_access
                    .denied
                    .roots()
                    .map(|roots| roots.len() as u64)
                    .unwrap_or(1);
                if merged {
                    if let Some(tree) = &mut self.analytics_tree {
                        let progress = crate::analytics::Progress::default();
                        if let Err(error) =
                            crate::analytics::fit_tree(tree, progress.node_budget(), &progress)
                        {
                            self.analytics_access.prior_incomplete = true;
                            self.analytics_issues.push(ScanIssue {
                                path,
                                detail: error.to_string(),
                            });
                        }
                    }
                }
                self.analytics_totals = None;
                self.analytics_invalidate();
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.analytics_access.partial = None;
                let denied = self.analytics_access.permission_denied;
                self.analytics_state = if denied > 0 || self.analytics_access.prior_incomplete {
                    StorageRunState::Partial
                } else {
                    StorageRunState::Complete
                };
                if denied == 0 && !self.analytics_access.prior_incomplete {
                    self.analytics_issues.clear();
                    self.analytics_suppressed_issues = 0;
                }
                self.update_analytics_access(denied);
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
    }

    pub(super) fn poll_analytics_access(&mut self) {
        self.poll_partial_analytics();
        let result = self
            .analytics_access
            .pending
            .as_ref()
            .map(|rx| rx.try_recv());
        match result {
            Some(Ok((root, result))) => {
                self.analytics_access.pending = None;
                self.analytics_access.message = Some(match result {
                    Ok(true) => {
                        if self.analytics_access.generation
                            == self.analytics_access.consent_generation
                            && self.analytics_scan.is_none()
                            && matches!(&self.analytics_source, Some(StorageScanSource::Local { root: current }) if current == &root)
                            && self.drive_consumer_available(
                                super::drive_snapshot::DriveConsumer::Analytics,
                                &root,
                                false,
                            )
                        {
                            match self
                                .analytics_access
                                .denied
                                .roots()
                                .map_err(|error| error.to_string())
                                .and_then(|roots| {
                                    super::partial_scan::analytics(
                                        root.clone(),
                                        self.analytics_access.generation,
                                        roots,
                                    )
                                }) {
                                Ok(worker) => {
                                    self.analytics_access.partial = Some(worker);
                                    self.analytics_state = StorageRunState::Running;
                                }
                                Err(error) => self.analytics_issues.push(ScanIssue {
                                    path: root.clone(),
                                    detail: error,
                                }),
                            }
                        }
                        if self.analytics_access.partial.is_some() {
                            format!("Leserechte für {root} verfügbar. Gesperrte Teilbäume werden aktualisiert.")
                        } else {
                            "Leserechte verfügbar. Die Analyse wurde inzwischen geändert oder die Teilscans konnten nicht starten; bisherige Ergebnisse bleiben erhalten.".into()
                        }
                    }
                    Ok(false) => {
                        "Rechteanfrage abgebrochen. Das bisherige Ergebnis bleibt erhalten.".into()
                    }
                    Err(error) => error,
                });
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.analytics_access.pending = None;
                self.analytics_access.message = Some("Rechteanfrage ohne Ergebnis beendet".into());
            }
            _ => {}
        }
    }
}

pub(super) fn access_ui(ui: &mut egui::Ui, state: &AnalyticsAccess) -> bool {
    let mut request = false;
    if state.permission_denied > 0 {
        ui.label(format!(
            "{} Zugriff(e) verweigert. Das Ergebnis ist nicht vollständig.",
            state.permission_denied
        ));
        if state.offer {
            ui.label("Mit Ihrer Zustimmung: geschützte Dateien in dieser Analyse erneut lesen. Die Ordnerrechte bleiben unverändert.");
            request = ui
                .add_enabled(
                    state.pending.is_none() && state.partial.is_none(),
                    egui::Button::new("Leserechte anfordern …"),
                )
                .clicked();
        } else {
            ui.label("Verbleibende Sperren benötigen ggf. Rechte beim Dateisystem oder Anbieter; lokale Administratorrechte lösen nicht jede Sperre.");
        }
    }
    if state.pending.is_some() || state.partial.is_some() {
        ui.spinner();
        ui.label(if state.partial.is_some() {
            "Gesperrte Teilbäume werden erneut gelesen …"
        } else {
            "Windows-Rechteanfrage läuft …"
        });
        if let Some(worker) = &state.partial {
            if ui.button("Teilscans abbrechen").clicked() {
                worker
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    if let Some(message) = &state.message {
        ui.label(message);
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analytics_access_task_remote_access_never_requests_local_privileges() {
        let source = StorageScanSource::remote(
            std::sync::Arc::new(crate::vfs::LocalBackend::new("/")),
            "/remote",
            "fixture",
        );
        assert!(!access_offer(5, Some(&source), |_| panic!(
            "remote scan must not query local authority"
        )));
        let local = StorageScanSource::local("C:/");
        assert!(access_offer(1, Some(&local), |root| root == "C:/"));
        assert!(!access_offer(0, Some(&local), |_| panic!("no denial")));
        assert!(!should_log_failure(StorageRunState::Partial, false));
        assert!(!should_log_failure(StorageRunState::Failed, true));
        assert!(should_log_failure(StorageRunState::Failed, false));
    }
    #[test]
    fn analytics_access_task_report_lists_each_retained_path_and_omissions() {
        let issues: Vec<_> = (0..24)
            .map(|index| ScanIssue {
                path: format!("C:/blocked-{index}/"),
                detail: "Zugriff verweigert".into(),
            })
            .collect();
        let report = issue_report(&issues, 3, 27);
        for issue in &issues {
            assert!(report.contains(&issue.path));
        }
        assert!(report.contains("3 weitere Probleme"));
        assert!(report.contains("27 Zugriff verweigert"));
    }
    #[test]
    fn analytics_access_task_new_scan_resets_prompt_without_canceling_consented_launch() {
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let mut state = AnalyticsAccess {
            permission_denied: 3,
            offer: true,
            pending: Some(rx),
            message: Some("previous".into()),
            ..Default::default()
        };
        state.reset_scan();
        assert_eq!(state.permission_denied, 0);
        assert!(!state.offer);
        assert!(state.message.is_none());
        assert!(state.pending.is_some());
    }
}
