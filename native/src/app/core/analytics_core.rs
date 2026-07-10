use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn build_summary(&self) -> SummaryData {
        let mut files = 0u64;
        let mut dirs = 0u64;
        let mut bytes = 0u64;
        let mut by_ext: std::collections::HashMap<&str, (u64, u64)> =
            std::collections::HashMap::new();
        let mut oldest = i64::MAX;
        let mut newest = 0i64;
        let mut top: Vec<&FileEntry> = Vec::new();

        for &(i, _) in &self.view {
            let e = &self.entries[i];
            if e.is_dir {
                dirs += 1;
            } else {
                files += 1;
                bytes += e.size;
                let k = if e.ext.is_empty() {
                    "(none)"
                } else {
                    e.ext.as_ref()
                };
                let entry = by_ext.entry(k).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += e.size;
                if e.mtime_ms != 0 && e.mtime_ms < oldest {
                    oldest = e.mtime_ms;
                }
                if e.mtime_ms > newest {
                    newest = e.mtime_ms;
                }
                if top.len() < 10 {
                    top.push(e);
                    top.sort_by_key(|entry| std::cmp::Reverse(entry.size));
                } else if top.last().map(|last| e.size > last.size).unwrap_or(false) {
                    if let Some(last) = top.last_mut() {
                        *last = e;
                    }
                    top.sort_by_key(|entry| std::cmp::Reverse(entry.size));
                }
            }
        }

        let mut by_ext_v: Vec<(String, u64, u64)> = by_ext
            .into_iter()
            .map(|(k, (c, b))| (k.to_string(), c, b))
            .collect();
        by_ext_v.sort_by_key(|entry| std::cmp::Reverse(entry.2));
        by_ext_v.truncate(15);

        SummaryData {
            files,
            dirs,
            bytes,
            oldest,
            newest,
            by_ext: by_ext_v,
            top: top
                .into_iter()
                .map(|e| (e.name.to_string(), e.path.to_string(), e.size))
                .collect(),
        }
    }

    /// The tree node at the current drill focus.
    pub(in crate::app) fn analytics_focus_node(&self) -> Option<&crate::analytics::SizeNode> {
        let mut node = self.analytics_tree.as_ref()?;
        for seg in &self.analytics_focus {
            node = node
                .children
                .iter()
                .find(|c| c.is_dir && &*c.name == seg.as_str())?;
        }
        Some(node)
    }

    /// Full `/`-path of the current drill focus.
    pub(in crate::app) fn analytics_focus_path(&self) -> String {
        let root = self.analytics_root();
        if self.analytics_focus.is_empty() {
            root.to_string()
        } else {
            format!(
                "{}/{}",
                root.trim_end_matches('/'),
                self.analytics_focus.join("/")
            )
        }
    }

    pub(in crate::app) fn analytics_root(&self) -> &str {
        self.analytics_source
            .as_ref()
            .map(StorageScanSource::root)
            .unwrap_or("")
    }

    /// Default scan target: the DRIVE ROOT of the current folder (WizTree-style
    /// whole-drive view) — never the app's own folder. Falls back to the current
    /// root for UNC / non-drive paths.
    pub(in crate::app) fn analytics_default_root(&self) -> String {
        let normalized = self.root_path.replace('\\', "/");
        if normalized == "/" {
            return normalized;
        }
        let rp = normalized.trim_end_matches('/');
        let b = rp.as_bytes();
        if b.len() >= 2 && b[1] == b':' {
            format!("{}:/", b[0] as char)
        } else {
            rp.to_string()
        }
    }

    /// Map a full `/`-path back to focus segments relative to the scanned root.
    pub(in crate::app) fn analytics_path_to_focus(&self, full: &str) -> Vec<String> {
        let root = self.analytics_root().trim_end_matches('/');
        let full = full.trim_end_matches('/');
        let rest = if full == root {
            ""
        } else {
            full.strip_prefix(root)
                .filter(|rest| rest.starts_with('/'))
                .unwrap_or("")
                .trim_start_matches('/')
        };
        if rest.is_empty() {
            Vec::new()
        } else {
            rest.split('/').map(|s| s.to_string()).collect()
        }
    }

    /// Invalidate the cached treemap cells + counts (after a drill / new tree).
    pub(in crate::app) fn analytics_invalidate(&mut self) {
        self.analytics_cells.clear();
        self.analytics_cells_rect = egui::Rect::ZERO;
        self.analytics_counts = None;
    }

    /// Kick off a dedicated low-memory size scan of `root_path` on a background
    /// thread; the result lands via `poll_analytics_scan`.
    pub(in crate::app) fn start_analytics_scan(&mut self, root_path: String) {
        self.start_analytics_source(StorageScanSource::local(root_path));
    }

    pub(in crate::app) fn start_analytics_source(&mut self, source: StorageScanSource) {
        self.cancel_analytics_worker();
        if source.root().is_empty() {
            let detail = "Leeres Scan-Ziel".to_string();
            self.analytics_source = None;
            self.analytics_state = StorageRunState::Failed;
            self.analytics_tree = None;
            self.analytics_issues = vec![crate::analytics::ScanIssue {
                path: String::new(),
                detail: detail.clone(),
            }];
            self.analytics_suppressed_issues = 0;
            self.push_app_error("Speicher-Analyse", detail);
            return;
        }
        let p = crate::analytics::Progress::default();
        let (tx, rx) = crossbeam_channel::unbounded();
        let p2 = p.clone();
        // A bare drive letter ("C:") must become a root ("C:\") or read_dir
        // would target the drive's *current directory* instead of its root.
        let worker_source = source.clone();
        let spawn = std::thread::Builder::new()
            .name("storage-analytics".into())
            .spawn(move || {
                let outcome = scan_storage_source(worker_source, &p2);
                let _ = tx.send(outcome);
            });
        self.analytics_source = Some(source.clone());
        self.analytics_focus.clear();
        self.analytics_tree = None;
        self.analytics_issues.clear();
        self.analytics_suppressed_issues = 0;
        self.analytics_invalidate();
        match spawn {
            Ok(_) => {
                self.analytics_scan = Some(AnalyticsScan {
                    rx,
                    progress: p,
                    root: source.display(),
                    started: Instant::now(),
                });
                self.analytics_state = StorageRunState::Running;
            }
            Err(error) => {
                let detail = format!("Scan-Thread konnte nicht starten: {error}");
                self.analytics_state = StorageRunState::Failed;
                self.analytics_issues.push(crate::analytics::ScanIssue {
                    path: source.root().to_string(),
                    detail: detail.clone(),
                });
                self.push_app_error("Speicher-Analyse", detail);
            }
        }
    }

    /// Kick off an analytics scan of a REMOTE folder via its VFS backend
    /// (SFTP/FTP/WebDAV/Drive). Serial + network-bound, so slower than local.
    pub(in crate::app) fn start_analytics_scan_remote(
        &mut self,
        backend: crate::vfs::BackendHandle,
        root: String,
        label: String,
    ) {
        self.start_analytics_source(StorageScanSource::remote(backend, root, label));
    }

    /// Drain a finished analytics scan into the tree (called each frame).
    pub(in crate::app) fn poll_analytics_scan(&mut self) {
        let message = self.analytics_scan.as_ref().map(|scan| scan.rx.try_recv());
        match message {
            Some(Ok(outcome)) => {
                self.analytics_scan = None;
                self.analytics_state = outcome.status.into();
                self.analytics_tree = outcome.tree;
                self.analytics_issues = outcome.issues;
                self.analytics_suppressed_issues = outcome.suppressed_issues;
                self.analytics_invalidate();
                self.log_analytics_outcome();
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.analytics_scan = None;
                self.analytics_state = StorageRunState::Failed;
                let detail = "Scan-Thread wurde ohne Ergebnis beendet".to_string();
                self.analytics_issues.push(crate::analytics::ScanIssue {
                    path: self.analytics_root().to_string(),
                    detail: detail.clone(),
                });
                self.push_app_error("Speicher-Analyse", detail);
            }
            _ => {}
        }
    }

    pub(in crate::app) fn cancel_analytics_scan(&mut self) {
        if self.cancel_analytics_worker() {
            self.analytics_state = StorageRunState::Canceled;
            self.analytics_issues.clear();
            self.analytics_suppressed_issues = 0;
        }
    }

    fn cancel_analytics_worker(&mut self) -> bool {
        if let Some(scan) = self.analytics_scan.take() {
            scan.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn log_analytics_outcome(&mut self) {
        if !matches!(
            self.analytics_state,
            StorageRunState::Partial | StorageRunState::Failed
        ) {
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

    pub(in crate::app) fn navigate_storage_source(
        &mut self,
        source: &StorageScanSource,
        target: &str,
    ) {
        match source {
            StorageScanSource::Local { .. } => self.remote = None,
            StorageScanSource::Remote { backend, label, .. } => {
                let same_backend = self
                    .remote
                    .as_ref()
                    .is_some_and(|remote| Arc::ptr_eq(&remote.backend, backend));
                if !same_backend {
                    self.remote = Some(crate::connect::RemoteState {
                        backend: backend.clone(),
                        label: label.clone(),
                        agent_version: None,
                        zip_return: None,
                        sftp: None,
                        account: None,
                        endpoint_prefix: None,
                    });
                }
            }
        }
        let native = target.replace('/', std::path::MAIN_SEPARATOR_STR);
        self.start_scan(PathBuf::from(native));
    }

    pub(in crate::app) fn ui_summary(&mut self, ui: &mut egui::Ui) {
        if self.summary_cache.is_none() {
            self.summary_cache = Some(self.build_summary());
        }
        let Some(s) = self.summary_cache.as_ref() else {
            return;
        };

        ui.heading("Zusammenfassung");
        ui.add_space(4.0);
        egui::Grid::new("summary_kv")
            .num_columns(2)
            .striped(false)
            .show(ui, |ui| {
                ui.label("Dateien");
                ui.label(format!("{}", s.files));
                ui.end_row();
                ui.label("Ordner");
                ui.label(format!("{}", s.dirs));
                ui.end_row();
                ui.label("Gesamtgröße");
                ui.label(format_bytes(s.bytes));
                ui.end_row();
                if s.oldest != i64::MAX {
                    ui.label("Älteste");
                    ui.label(format_date(s.oldest));
                    ui.end_row();
                }
                if s.newest > 0 {
                    ui.label("Neueste");
                    ui.label(format_date(s.newest));
                    ui.end_row();
                }
            });

        ui.add_space(8.0);
        ui.label(
            RichText::new("TOP-DATEITYPEN")
                .small()
                .color(Color32::from_gray(140)),
        );
        for (k, count, bytes) in &s.by_ext {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(80, 140, 255), RichText::new(k).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format_bytes(*bytes));
                    ui.label(format!("{} ×", count));
                });
            });
        }

        ui.add_space(8.0);
        ui.label(
            RichText::new("GRÖSSTE DATEIEN")
                .small()
                .color(Color32::from_gray(140)),
        );
        for (name, path, size) in s.top.iter().take(10) {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(80, 140, 255), format_bytes(*size));
                ui.add(egui::Label::new(name).truncate())
                    .on_hover_text(path);
            });
        }
    }

    /// Drive used/total for the drive that `root` lives on.
    pub(in crate::app) fn drive_usage(&self, root: &str) -> Option<(u64, u64)> {
        let dl = root.get(0..2)?.to_ascii_uppercase();
        for (r, free, total) in &self.drive_info {
            if *total > 0 && r.to_ascii_uppercase().starts_with(&dl) {
                return Some((total.saturating_sub(*free), *total));
            }
        }
        None
    }
}

fn scan_storage_source(
    source: StorageScanSource,
    progress: &crate::analytics::Progress,
) -> crate::analytics::ScanOutcome {
    match source {
        StorageScanSource::Local { root } => {
            let native = root.replace('/', std::path::MAIN_SEPARATOR_STR);
            crate::analytics::scan(&PathBuf::from(native), progress)
        }
        StorageScanSource::Remote { backend, root, .. } => {
            backend.invalidate_cache();
            if !backend.supports_walk_tree() {
                return crate::analytics::scan_backend(&*backend, &root, progress);
            }
            let live = progress.clone();
            let on_progress = move |files: u64, bytes: u64| -> bool {
                live.files
                    .store(files, std::sync::atomic::Ordering::Relaxed);
                live.bytes
                    .store(bytes, std::sync::atomic::Ordering::Relaxed);
                !live.cancel.load(std::sync::atomic::Ordering::Relaxed)
            };
            match backend.walk_tree(&root, &on_progress) {
                Ok(Some(tree)) if !progress.cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                    crate::analytics::ScanOutcome::complete(crate::analytics::from_wire(tree))
                }
                Ok(Some(_)) => crate::analytics::ScanOutcome::canceled(),
                Ok(None) if progress.cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                    crate::analytics::ScanOutcome::canceled()
                }
                Ok(None) => crate::analytics::scan_backend(&*backend, &root, progress),
                Err(_) if progress.cancel.load(std::sync::atomic::Ordering::Relaxed) => {
                    crate::analytics::ScanOutcome::canceled()
                }
                Err(error) => crate::analytics::ScanOutcome::failed(root, error.to_string()),
            }
        }
    }
}
