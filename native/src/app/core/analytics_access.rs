use super::{App, StorageRunState, StorageScanSource};
use crate::analytics::ScanIssue;
use eframe::egui;

#[derive(Default)]
pub(super) struct AnalyticsAccess {
    pub permission_denied: u64,
    pub offer: bool,
    pub pending: Option<crossbeam_channel::Receiver<(String, Result<bool, String>)>>,
    pub message: Option<String>,
}

impl AnalyticsAccess {
    pub(super) fn reset_scan(&mut self) {
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

pub(super) fn issues_ui(ui: &mut egui::Ui, issues: &[ScanIssue], suppressed: u64, denied: u64) {
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
                .id_salt("analytics_read_issues_scroll")
                .max_height(180.0)
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
    pub(super) fn update_analytics_access(&mut self, denied: u64) {
        self.analytics_access.permission_denied = denied;
        self.analytics_access.offer = access_offer(
            denied,
            self.analytics_source.as_ref(),
            crate::analytics::can_request_elevation,
        );
    }

    pub(super) fn request_analytics_access(&mut self) {
        if !self.analytics_access.offer || self.analytics_access.pending.is_some() {
            return;
        }
        let Some(StorageScanSource::Local { root }) = &self.analytics_source else {
            return;
        };
        let root = root.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        match std::thread::Builder::new()
            .name("analytics-consent".into())
            .spawn(move || {
                let result = crate::analytics::launch_elevated_analysis(&root);
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

    pub(super) fn poll_analytics_access(&mut self) {
        let result = self
            .analytics_access
            .pending
            .as_ref()
            .map(|rx| rx.try_recv());
        match result {
            Some(Ok((root, result))) => {
                self.analytics_access.pending = None;
                self.analytics_access.message = Some(match result {
                    Ok(true) => format!("Administrator-Analyse für {root} in einem eigenen Fenster gestartet. Dieses Ergebnis bleibt erhalten."),
                    Ok(false) => "Rechteanfrage abgebrochen. Das bisherige Ergebnis bleibt erhalten.".into(),
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
            ui.label("Mit Ihrer Zustimmung: denselben Pfad in einem separaten Administrator-Analysefenster erneut lesen. Keine Änderung von Besitzrechten oder Berechtigungen.");
            request = ui
                .add_enabled(
                    state.pending.is_none(),
                    egui::Button::new("Administratorrechte anfordern …"),
                )
                .clicked();
        } else {
            ui.label("Verbleibende Sperren benötigen ggf. Rechte beim Dateisystem oder Anbieter; lokale Administratorrechte lösen nicht jede Sperre.");
        }
    }
    if state.pending.is_some() {
        ui.spinner();
        ui.label("Windows-Rechteanfrage läuft …");
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
        };
        state.reset_scan();
        assert_eq!(state.permission_denied, 0);
        assert!(!state.offer);
        assert!(state.message.is_none());
        assert!(state.pending.is_some());
    }
}
