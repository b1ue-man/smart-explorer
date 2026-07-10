use super::prelude::*;
use super::*;
use crate::app::delete_worker::DeleteReporter;
use std::sync::atomic::{AtomicBool, Ordering};

impl App {
    pub(in crate::app) fn reclaim_options(&self) -> crate::analytics::ReclaimOptions {
        crate::analytics::ReclaimOptions {
            large_min_bytes: (self.reclaim_large_min_gb.max(0.01) * 1024.0 * 1024.0 * 1024.0)
                as u64,
            stale_days: self.reclaim_stale_days.max(1),
            max_items: 200,
            duplicate_min_bytes: 1024 * 1024,
            partial_fingerprint_bytes: 64 * 1024,
        }
    }

    pub(in crate::app) fn start_reclaim_scan(&mut self, root_path: String) {
        self.start_reclaim_source(StorageScanSource::local(root_path));
    }

    pub(in crate::app) fn start_reclaim_source(&mut self, source: StorageScanSource) {
        self.cancel_reclaim_worker();
        if source.root().is_empty() {
            let detail = "Leeres Reclaim-Ziel".to_string();
            self.reclaim_source = None;
            self.reclaim_state = StorageRunState::Failed;
            self.reclaim_report = None;
            self.reclaim_issues = vec![detail.clone()];
            self.reclaim_suppressed_issues = 0;
            self.push_app_error("Find & Reclaim", detail);
            return;
        }
        let progress = crate::analytics::ReclaimProgress::default();
        let opts = self.reclaim_options();
        let (tx, rx) = unbounded();
        let p2 = progress.clone();
        let worker_source = source.clone();
        let spawn = std::thread::Builder::new()
            .name("reclaim-scan".into())
            .spawn(move || {
                let report = match worker_source {
                    StorageScanSource::Local { root } => {
                        let native = root.replace('/', std::path::MAIN_SEPARATOR_STR);
                        crate::analytics::scan_reclaim(&PathBuf::from(native), &p2, &opts)
                    }
                    StorageScanSource::Remote { backend, root, .. } => {
                        backend.invalidate_cache();
                        crate::analytics::scan_reclaim_backend(backend, &root, &p2, &opts)
                    }
                };
                let outcome = reclaim_scan_outcome(report, &p2);
                let _ = tx.send(outcome);
            });
        self.reclaim_source = Some(source.clone());
        self.reclaim_report = None;
        self.reclaim_selected.clear();
        self.reclaim_issues.clear();
        self.reclaim_suppressed_issues = 0;
        match spawn {
            Ok(_) => {
                self.reclaim_scan = Some(ReclaimScan {
                    rx,
                    progress,
                    root: source.display(),
                    started: Instant::now(),
                });
                self.reclaim_state = StorageRunState::Running;
            }
            Err(error) => {
                let detail = format!("Reclaim-Thread konnte nicht starten: {error}");
                self.reclaim_state = StorageRunState::Failed;
                self.reclaim_issues.push(detail.clone());
                self.push_app_error("Find & Reclaim", detail);
            }
        }
    }

    pub(in crate::app) fn start_reclaim_scan_remote(
        &mut self,
        backend: crate::vfs::BackendHandle,
        root: String,
        label: String,
    ) {
        self.start_reclaim_source(StorageScanSource::remote(backend, root, label));
    }

    pub(in crate::app) fn poll_reclaim_scan(&mut self) {
        let message = self.reclaim_scan.as_ref().map(|scan| scan.rx.try_recv());
        match message {
            Some(Ok(outcome)) => {
                self.reclaim_scan = None;
                self.reclaim_state = outcome.status;
                self.reclaim_report = outcome.report;
                self.reclaim_issues = outcome.issues;
                self.reclaim_suppressed_issues = outcome.suppressed_issues;
                self.reclaim_selected.clear();
                self.log_reclaim_outcome();
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.reclaim_scan = None;
                self.reclaim_state = StorageRunState::Failed;
                let detail = "Reclaim-Thread wurde ohne Ergebnis beendet".to_string();
                self.reclaim_issues.push(detail.clone());
                self.push_app_error("Find & Reclaim", detail);
            }
            _ => {}
        }
    }

    pub(in crate::app) fn cancel_reclaim_scan(&mut self) {
        if self.cancel_reclaim_worker() {
            self.reclaim_state = StorageRunState::Canceled;
            self.reclaim_issues.clear();
            self.reclaim_suppressed_issues = 0;
        }
    }

    fn cancel_reclaim_worker(&mut self) -> bool {
        if let Some(scan) = self.reclaim_scan.take() {
            scan.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn log_reclaim_outcome(&mut self) {
        if !matches!(
            self.reclaim_state,
            StorageRunState::Partial | StorageRunState::Failed
        ) {
            return;
        }
        let first = self
            .reclaim_issues
            .first()
            .cloned()
            .unwrap_or_else(|| "Unbekannter Scan-Fehler".to_string());
        let count = self.reclaim_issues.len() as u64 + self.reclaim_suppressed_issues;
        self.push_app_error(
            "Find & Reclaim",
            format!("{count} Leseproblem(e); erstes Problem: {first}"),
        );
    }

    pub(in crate::app) fn select_reclaim_duplicate_copies(&mut self) {
        self.reclaim_selected.clear();
        if let Some(report) = &self.reclaim_report {
            for group in &report.duplicate_groups {
                for item in group.items.iter().skip(1) {
                    self.reclaim_selected.insert(item.path.clone());
                }
            }
        }
    }

    pub(in crate::app) fn trash_reclaim_selected(&mut self) {
        if self.reclaim_selected.is_empty() {
            return;
        }
        if self.trash_rx.is_some() || self.trash_worker.is_some() {
            self.error_msg = Some("Ein Löschvorgang läuft bereits.".to_string());
            return;
        }
        let Some(report_snapshot) = self.reclaim_report.clone() else {
            return;
        };
        if report_snapshot.is_remote {
            self.error_msg = Some("Remote-Reclaim ist in diesem Release read-only.".to_string());
            return;
        }
        let paths = self.reclaim_selected_paths_expanded();
        if paths.is_empty() {
            return;
        }
        let plan = crate::analytics::prepare_reclaim_trash_plan(&report_snapshot, &paths);
        if plan.delete_paths.is_empty() {
            self.error_msg = Some(format!(
                "Keine sicher verschiebbaren Eintraege. {} uebersprungen.",
                plan.skipped_paths.len()
            ));
            return;
        }
        let bytes = plan.estimated_bytes;
        let mut detail = format!(
            "{} Eintrag/Eintraege ({}) in den Papierkorb verschieben?",
            plan.delete_paths.len(),
            format_bytes(bytes)
        );
        if !plan.verified_duplicate_paths.is_empty() {
            detail.push_str(&format!(
                "\n{} Duplikatkopie(n) byteweise verifiziert.",
                plan.verified_duplicate_paths.len()
            ));
        }
        if !plan.skipped_paths.is_empty() {
            detail.push_str(&format!(
                "\n{} Eintrag/Eintraege wegen Aenderung oder fehlender Verifikation uebersprungen.",
                plan.skipped_paths.len()
            ));
        }
        if !plan.risky_paths.is_empty() {
            detail.push_str(&format!(
                "\n{} riskante Review-Auswahl(en) enthalten.",
                plan.risky_paths.len()
            ));
        }
        if !confirm_yes_no("In Papierkorb verschieben", &detail) {
            return;
        }
        let delete_paths = plan.delete_paths.clone();
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let root = report_snapshot.root;
        let selected = paths;
        let journal_plan = plan.clone();
        let attempted = delete_paths.len();
        let mut initial =
            DeleteProgress::new(DeleteKind::Recycle, DeleteOrigin::Reclaim, attempted);
        initial.phase = DeletePhase::Applying;
        let worker_progress = initial.clone();
        let spawn = std::thread::Builder::new()
            .name("reclaim-trash".into())
            .spawn(move || {
                let mut outcome =
                    DeleteOutcome::new(DeleteKind::Recycle, DeleteOrigin::Reclaim, attempted);
                let mut reporter = DeleteReporter::new(tx, worker_cancel.clone(), worker_progress);
                for display in delete_paths {
                    if worker_cancel.load(Ordering::Acquire)
                        || !reporter.begin_opaque_apply(&display)
                    {
                        outcome.canceled = true;
                        break;
                    }
                    outcome.entries_planned = outcome.entries_planned.saturating_add(1);
                    let native = PathBuf::from(display.replace('/', std::path::MAIN_SEPARATOR_STR));
                    match trash::delete(native) {
                        Ok(()) => {
                            outcome.entries_deleted = outcome.entries_deleted.saturating_add(1);
                            outcome.record_success(display);
                            reporter.finish_target(true, true);
                        }
                        Err(error) => {
                            outcome.partial_mutation = true;
                            outcome.record_error(display, error.to_string());
                            reporter.finish_target(false, false);
                        }
                    }
                }
                if worker_cancel.load(Ordering::Acquire) && outcome.processed < outcome.attempted {
                    outcome.canceled = true;
                }
                if let Err(error) =
                    append_reclaim_journal(&root, &selected, &journal_plan, &outcome)
                {
                    outcome.record_aux_error("Reclaim-Journal".to_string(), error.to_string());
                }
                reporter.finish(outcome);
            });
        self.install_delete_worker(spawn, rx, cancel, initial, DeleteOrigin::Reclaim);
    }

    fn reclaim_selected_paths_expanded(&self) -> Vec<String> {
        let Some(report) = &self.reclaim_report else {
            return Vec::new();
        };
        let selected: std::collections::HashSet<&str> =
            self.reclaim_selected.iter().map(String::as_str).collect();
        let mut out = Vec::new();
        for item in reclaim_items(report) {
            if selected.contains(item.path.as_str())
                || selected.iter().any(|p| {
                    item.path
                        .starts_with(&format!("{}/", p.trim_end_matches('/')))
                })
            {
                out.push(item.path.clone());
            }
        }
        out.extend(self.reclaim_selected.iter().cloned());
        out.sort();
        out.dedup();
        out
    }
}

fn reclaim_scan_outcome(
    report: crate::analytics::ReclaimReport,
    progress: &crate::analytics::ReclaimProgress,
) -> ReclaimScanOutcome {
    if progress.cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return ReclaimScanOutcome {
            report: None,
            status: StorageRunState::Canceled,
            issues: Vec::new(),
            suppressed_issues: 0,
        };
    }
    let mut issues = report.errors.clone();
    if let Some(root_error) = &report.root_error {
        if !issues.contains(root_error) {
            issues.insert(0, root_error.clone());
        }
    }
    let status = if report.root_error.is_some() {
        StorageRunState::Failed
    } else if issues.is_empty() && report.suppressed_errors == 0 {
        StorageRunState::Complete
    } else {
        StorageRunState::Partial
    };
    let suppressed_issues = report.suppressed_errors;
    ReclaimScanOutcome {
        report: (status != StorageRunState::Failed).then_some(report),
        status,
        issues,
        suppressed_issues,
    }
}

fn reclaim_items(report: &crate::analytics::ReclaimReport) -> Vec<&crate::analytics::ReclaimItem> {
    let mut out = Vec::new();
    out.extend(report.large_files.iter());
    out.extend(report.stale_files.iter());
    out.extend(report.empty_files.iter());
    out.extend(report.empty_dirs.iter());
    out.extend(report.cleanup.iter());
    for g in &report.duplicate_groups {
        out.extend(g.items.iter());
    }
    out
}

fn append_reclaim_journal(
    root: &str,
    selected: &[String],
    plan: &crate::analytics::ReclaimTrashPlan,
    outcome: &DeleteOutcome,
) -> std::io::Result<()> {
    let dir = crate::support_dirs::app_data_dir().join("reclaim");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("actions.jsonl");
    let ts = chrono::Local::now().to_rfc3339();
    let value = serde_json::json!({
        "ts": ts,
        "root": root,
        "selected": selected,
        "delete_paths": &plan.delete_paths,
        "verified_duplicate_paths": &plan.verified_duplicate_paths,
        "skipped_paths": &plan.skipped_paths,
        "risky_paths": &plan.risky_paths,
        "estimated_bytes": plan.estimated_bytes,
        "attempted": outcome.attempted,
        "processed": outcome.processed,
        "succeeded_paths": &outcome.succeeded_paths,
        "canceled": outcome.canceled,
        "entries_planned": outcome.entries_planned,
        "entries_deleted": outcome.entries_deleted,
        "partial_mutation": outcome.partial_mutation,
        "errors": &outcome.errors,
        "suppressed_errors": outcome.suppressed_errors,
    });
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    use std::io::Write;
    writeln!(file, "{value}")?;
    file.flush()
}
