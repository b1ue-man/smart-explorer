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
                let k = if e.ext.is_empty() { "(none)" } else { e.ext.as_ref() };
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
                    top.sort_by(|a, b| b.size.cmp(&a.size));
                } else if e.size > top.last().unwrap().size {
                    *top.last_mut().unwrap() = e;
                    top.sort_by(|a, b| b.size.cmp(&a.size));
                }
            }
        }

        let mut by_ext_v: Vec<(String, u64, u64)> = by_ext
            .into_iter()
            .map(|(k, (c, b))| (k.to_string(), c, b))
            .collect();
        by_ext_v.sort_by(|a, b| b.2.cmp(&a.2));
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
        let root = self.analytics_root_path.trim_end_matches('/');
        if self.analytics_focus.is_empty() {
            root.to_string()
        } else {
            format!("{}/{}", root, self.analytics_focus.join("/"))
        }
    }

    /// Default scan target: the DRIVE ROOT of the current folder (WizTree-style
    /// whole-drive view) — never the app's own folder. Falls back to the current
    /// root for UNC / non-drive paths.
    pub(in crate::app) fn analytics_default_root(&self) -> String {
        let rp = self.root_path.trim_end_matches('/');
        let b = rp.as_bytes();
        if b.len() >= 2 && b[1] == b':' {
            format!("{}:/", b[0] as char)
        } else {
            rp.to_string()
        }
    }

    /// Map a full `/`-path back to focus segments relative to the scanned root.
    pub(in crate::app) fn analytics_path_to_focus(&self, full: &str) -> Vec<String> {
        let root = self.analytics_root_path.trim_end_matches('/');
        let rest = full
            .strip_prefix(root)
            .unwrap_or("")
            .trim_start_matches('/');
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
        let norm = root_path.trim_end_matches('/').to_string();
        if norm.is_empty() {
            return;
        }
        if let Some(s) = &self.analytics_scan {
            s.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let p = crate::analytics::Progress::default();
        let (tx, rx) = crossbeam_channel::unbounded();
        let p2 = p.clone();
        // A bare drive letter ("C:") must become a root ("C:\") or read_dir
        // would target the drive's *current directory* instead of its root.
        let sep = std::path::MAIN_SEPARATOR;
        let mut native = norm.replace('/', std::path::MAIN_SEPARATOR_STR);
        if native.len() == 2 && native.as_bytes()[1] == b':' {
            native.push(sep);
        }
        let root_pb = PathBuf::from(native);
        std::thread::spawn(move || {
            let node = crate::analytics::scan(&root_pb, &p2);
            let _ = tx.send(node);
        });
        self.analytics_scan = Some(AnalyticsScan {
            rx,
            progress: p,
            root: norm.clone(),
            started: Instant::now(),
        });
        self.analytics_root_path = norm;
        self.analytics_backend = None;
        self.analytics_focus.clear();
        self.analytics_tree = None;
        self.analytics_invalidate();
    }

    /// Kick off an analytics scan of a REMOTE folder via its VFS backend
    /// (SFTP/FTP/WebDAV/Drive). Serial + network-bound, so slower than local.
    pub(in crate::app) fn start_analytics_scan_remote(
        &mut self,
        backend: crate::vfs::BackendHandle,
        root: String,
        label: String,
    ) {
        let t = root.trim_end_matches('/');
        let norm = if t.is_empty() { "/".to_string() } else { t.to_string() };
        if let Some(s) = &self.analytics_scan {
            s.progress
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let p = crate::analytics::Progress::default();
        let (tx, rx) = crossbeam_channel::unbounded();
        let p2 = p.clone();
        let be = backend.clone();
        let scan_root = norm.clone();
        std::thread::spawn(move || {
            // If the backend has a server-side agent, let IT walk the whole tree
            // (one request, no per-dir round-trip) while streaming live progress
            // into `p2`; else walk client-side.
            let node = if be.supports_walk_tree() {
                let prog = p2.clone();
                let on_progress = move |files: u64, bytes: u64| -> bool {
                    prog.files.store(files, std::sync::atomic::Ordering::Relaxed);
                    prog.bytes.store(bytes, std::sync::atomic::Ordering::Relaxed);
                    !prog.cancel.load(std::sync::atomic::Ordering::Relaxed)
                };
                match be.walk_tree(&scan_root, &on_progress) {
                    Some(w) => crate::analytics::from_wire(w),
                    None => crate::analytics::scan_backend(&*be, &scan_root, &p2),
                }
            } else {
                crate::analytics::scan_backend(&*be, &scan_root, &p2)
            };
            let _ = tx.send(node);
        });
        self.analytics_scan = Some(AnalyticsScan {
            rx,
            progress: p,
            root: if label.is_empty() { norm.clone() } else { format!("{} · {}", label, norm) },
            started: Instant::now(),
        });
        self.analytics_root_path = norm;
        self.analytics_backend = Some(backend);
        self.analytics_focus.clear();
        self.analytics_tree = None;
        self.analytics_invalidate();
    }

    /// Drain a finished analytics scan into the tree (called each frame).
    pub(in crate::app) fn poll_analytics_scan(&mut self) {
        let mut got = None;
        if let Some(scan) = &self.analytics_scan {
            if let Ok(node) = scan.rx.try_recv() {
                got = Some(node);
            }
        }
        if let Some(node) = got {
            self.analytics_tree = Some(node);
            self.analytics_scan = None;
            self.analytics_invalidate();
        }
    }

    pub(in crate::app) fn ui_summary(&mut self, ui: &mut egui::Ui) {
        if self.summary_cache.is_none() {
            self.summary_cache = Some(self.build_summary());
        }
        let s = self.summary_cache.as_ref().unwrap();

        ui.heading("Zusammenfassung");
        ui.add_space(4.0);
        egui::Grid::new("summary_kv").num_columns(2).striped(false).show(ui, |ui| {
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
        ui.label(RichText::new("TOP-DATEITYPEN").small().color(Color32::from_gray(140)));
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
        ui.label(RichText::new("GRÖSSTE DATEIEN").small().color(Color32::from_gray(140)));
        for (name, path, size) in s.top.iter().take(10) {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(80, 140, 255), format_bytes(*size));
                ui.add(egui::Label::new(name).truncate()).on_hover_text(path);
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
