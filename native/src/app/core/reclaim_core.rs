use super::prelude::*;
use super::*;

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
        let norm = root_path.trim_end_matches('/').to_string();
        if norm.is_empty() || crate::connect::is_remote_url(&norm) {
            self.error_msg = Some("Aufraeumen scannt in diesem Build lokale Ordner.".to_string());
            return;
        }
        if let Some(s) = &self.reclaim_scan {
            s.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let progress = crate::analytics::ReclaimProgress::default();
        let opts = self.reclaim_options();
        let (tx, rx) = unbounded();
        let p2 = progress.clone();
        let mut native = norm.replace('/', std::path::MAIN_SEPARATOR_STR);
        if native.len() == 2 && native.as_bytes()[1] == b':' {
            native.push(std::path::MAIN_SEPARATOR);
        }
        let root = PathBuf::from(native);
        std::thread::Builder::new()
            .name("reclaim-scan".into())
            .spawn(move || {
                let report = crate::analytics::scan_reclaim(&root, &p2, &opts);
                let _ = tx.send(report);
            })
            .ok();
        self.reclaim_scan = Some(ReclaimScan {
            rx,
            progress,
            root: norm,
            started: Instant::now(),
            cancel_requested: false,
        });
        self.reclaim_report = None;
        self.reclaim_selected.clear();
    }

    pub(in crate::app) fn start_reclaim_scan_remote(
        &mut self,
        backend: crate::vfs::BackendHandle,
        root: String,
        label: String,
    ) {
        let t = root.trim_end_matches('/');
        let norm = if t.is_empty() {
            "/".to_string()
        } else {
            t.to_string()
        };
        if let Some(s) = &self.reclaim_scan {
            s.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let progress = crate::analytics::ReclaimProgress::default();
        let opts = self.reclaim_options();
        let (tx, rx) = unbounded();
        let p2 = progress.clone();
        let be = backend.clone();
        let scan_root = norm.clone();
        std::thread::Builder::new()
            .name("reclaim-remote-scan".into())
            .spawn(move || {
                let report = crate::analytics::scan_reclaim_backend(be, &scan_root, &p2, &opts);
                let _ = tx.send(report);
            })
            .ok();
        self.reclaim_scan = Some(ReclaimScan {
            rx,
            progress,
            root: if label.is_empty() {
                norm
            } else {
                format!("{} · {}", label, norm)
            },
            started: Instant::now(),
            cancel_requested: false,
        });
        self.reclaim_report = None;
        self.reclaim_selected.clear();
    }

    pub(in crate::app) fn poll_reclaim_scan(&mut self) {
        let mut got = None;
        let mut canceled = false;
        if let Some(scan) = &self.reclaim_scan {
            canceled = scan.cancel_requested;
            if let Ok(report) = scan.rx.try_recv() {
                got = Some(report);
            }
        }
        if let Some(report) = got {
            if !canceled {
                self.reclaim_report = Some(report);
            }
            self.reclaim_scan = None;
            self.reclaim_selected.clear();
        }
    }

    pub(in crate::app) fn cancel_reclaim_scan(&mut self) {
        if let Some(scan) = &mut self.reclaim_scan {
            scan.cancel_requested = true;
            scan.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
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
        let native_paths: Vec<PathBuf> = delete_paths
            .iter()
            .map(|p| PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .collect();
        if let Some(report) = &mut self.reclaim_report {
            report.prune_paths(&delete_paths);
        }
        self.reclaim_selected.clear();

        let (tx, rx) = unbounded();
        self.trash_rx = Some(rx);
        let root = report_snapshot.root;
        let selected = paths;
        let journal_plan = plan.clone();
        std::thread::Builder::new()
            .name("reclaim-trash".into())
            .spawn(move || {
                let res = trash::delete_all(&native_paths);
                let err = res.err().map(|e| e.to_string());
                append_reclaim_journal(root, selected, journal_plan, err.as_deref());
                let _ = tx.send(err);
            })
            .ok();
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
    root: String,
    selected: Vec<String>,
    plan: crate::analytics::ReclaimTrashPlan,
    error: Option<&str>,
) {
    let dir = crate::support_dirs::app_data_dir().join("reclaim");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("actions.jsonl");
    let ts = chrono::Local::now().to_rfc3339();
    let value = serde_json::json!({
        "ts": ts,
        "root": root,
        "selected": selected,
        "delete_paths": plan.delete_paths,
        "verified_duplicate_paths": plan.verified_duplicate_paths,
        "skipped_paths": plan.skipped_paths,
        "risky_paths": plan.risky_paths,
        "estimated_bytes": plan.estimated_bytes,
        "result": error.unwrap_or("ok"),
    });
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{}", value);
    }
}
