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

    pub(in crate::app) fn reclaim_selected_bytes(&self) -> u64 {
        let Some(report) = &self.reclaim_report else {
            return 0;
        };
        let mut seen = std::collections::HashSet::new();
        let mut total = 0u64;
        for item in reclaim_items(report) {
            if self.reclaim_selected.contains(&item.path) && seen.insert(item.path.as_str()) {
                total = total.saturating_add(item.size);
            }
        }
        total
    }

    pub(in crate::app) fn trash_reclaim_selected(&mut self) {
        if self.reclaim_selected.is_empty() {
            return;
        }
        let paths = self.reclaim_selected_paths_expanded();
        if paths.is_empty() {
            return;
        }
        let bytes = self.reclaim_selected_bytes();
        if !confirm_yes_no(
            "In Papierkorb verschieben",
            &format!(
                "{} Eintrag/Eintraege ({}) in den Papierkorb verschieben?",
                paths.len(),
                format_bytes(bytes)
            ),
        ) {
            return;
        }
        let delete_paths = dedupe_nested_paths(&paths);
        let native_paths: Vec<PathBuf> = delete_paths
            .iter()
            .map(|p| PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .collect();
        if let Some(report) = &mut self.reclaim_report {
            report.prune_paths(&paths);
        }
        self.reclaim_selected.clear();

        let (tx, rx) = unbounded();
        self.trash_rx = Some(rx);
        std::thread::Builder::new()
            .name("reclaim-trash".into())
            .spawn(move || {
                let res = trash::delete_all(&native_paths);
                let _ = tx.send(res.err().map(|e| e.to_string()));
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

fn dedupe_nested_paths(paths: &[String]) -> Vec<String> {
    let mut sorted = paths.to_vec();
    sorted.sort_by_key(|p| p.matches('/').count());
    let mut out: Vec<String> = Vec::new();
    'next: for p in sorted {
        for kept in &out {
            if p == *kept || p.starts_with(&format!("{}/", kept.trim_end_matches('/'))) {
                continue 'next;
            }
        }
        out.push(p);
    }
    out
}
