use super::prelude::*;
use super::*;

impl App {
    #[cfg(windows)]
    pub(in crate::app) fn clipboard_copy_files(&mut self, cut: bool) {
        if self.selection.is_empty() {
            self.notice = Some((
                "Nichts ausgewählt — bitte erst Dateien markieren".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        // Remote selection → download the files to temp, then put those local
        // paths on the clipboard so they paste into Explorer (or back into us).
        if let Some(rs) = &self.remote {
            let files: Vec<(String, String)> = self
                .entries
                .iter()
                .filter(|e| !e.is_dir && self.selection.contains(&e.key()))
                .map(|e| (e.path.to_string(), e.name.to_string()))
                .collect();
            if files.is_empty() {
                self.notice = Some((
                    "Remote: nur Dateien können in die Zwischenablage kopiert werden (keine Ordner).".to_string(),
                    std::time::Instant::now(),
                ));
                return;
            }
            let backend = rs.backend.clone();
            let n = files.len();
            let (tx, rx) = unbounded();
            self.clip_download_rx = Some(rx);
            self.notice = Some((
                format!("⬇ Bereite {} Datei(en) für die Zwischenablage vor…", n),
                std::time::Instant::now(),
            ));
            std::thread::Builder::new()
                .name("clip-download".into())
                .spawn(move || {
                    let mut local = Vec::new();
                    for (path, name) in &files {
                        if let Ok(p) = download_to_temp(&*backend, path, name) {
                            local.push(p);
                        }
                    }
                    let _ = tx.send(local);
                })
                .ok();
            return;
        }
        let has_dir = self
            .entries
            .iter()
            .any(|e| e.is_dir && self.selection.contains(&e.key()));

        // Filter-aware copy: when a filter is active and folders are selected,
        // build a virtual-file data object so pasting (anywhere) recreates
        // only the matching files with their folder structure.
        if !cut && has_dir && self.filter_is_active() {
            let seeds: Vec<FileEntry> = self
                .entries
                .iter()
                .filter(|e| self.selection.contains(&e.key()))
                .cloned()
                .collect();
            let filter = self.filter.clone();
            let prefix = self.root_prefix();
            let (tx, rx) = unbounded();
            self.clip_prepare_rx = Some(rx);
            self.notice = Some((
                "Sammle gefilterte Dateien…".to_string(),
                std::time::Instant::now(),
            ));
            std::thread::Builder::new()
                .name("clip-prepare".into())
                .spawn(move || {
                    let cf = CompiledFilter::compile(&filter);
                    let mut out: Vec<crate::virtual_clipboard::VirtualFile> = Vec::new();
                    for e in &seeds {
                        if e.is_dir {
                            let parent_norm = e.parent.trim_end_matches('/');
                            let base = format!("{}/", parent_norm);
                            let sub = crate::scanner::collect_recursive(
                                &PathBuf::from(
                                    e.path.replace('/', std::path::MAIN_SEPARATOR_STR),
                                ),
                                false,
                                e.depth + 1,
                            );
                            for s in sub {
                                if !s.is_dir && cf.matches(&s, &prefix) {
                                    let rel = s
                                        .path
                                        .strip_prefix(base.as_str())
                                        .unwrap_or(s.name.as_ref())
                                        .to_string();
                                    out.push(crate::virtual_clipboard::VirtualFile {
                                        abs: s.path.replace('/', "\\"),
                                        rel,
                                        size: s.size,
                                        mtime_ms: s.mtime_ms,
                                    });
                                }
                            }
                        } else {
                            // Explicitly selected files always go along.
                            out.push(crate::virtual_clipboard::VirtualFile {
                                abs: e.path.replace('/', "\\"),
                                rel: e.name.to_string(),
                                size: e.size,
                                mtime_ms: e.mtime_ms,
                            });
                        }
                    }
                    let _ = tx.send(out);
                })
                .ok();
            return;
        }

        // Plain CF_HDROP path (no filter, or cut, or files only).
        let paths: Vec<String> = self.selection.iter().map(|k| sel_key_path(k).replace('/', "\\")).collect();
        let effect = if cut {
            crate::shell_clipboard::DROPEFFECT_MOVE
        } else {
            crate::shell_clipboard::DROPEFFECT_COPY
        };
        match crate::shell_clipboard::write_files(&paths, effect) {
            Ok(_) => {
                self.virtual_clip = None;
                let hint = if cut && has_dir && self.filter_is_active() {
                    " — Hinweis: Ausschneiden überträgt ganze Ordner, Filter gelten dabei nicht"
                } else {
                    ""
                };
                self.notice = Some((
                    format!(
                        "✓ {} Datei(en) {} — in Explorer einfügbar mit Ctrl+V{}",
                        paths.len(),
                        if cut { "ausgeschnitten" } else { "kopiert" },
                        hint
                    ),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.error_msg = Some(format!("Zwischenablage: {}", e));
            }
        }
    }

    #[cfg(windows)]
    pub(in crate::app) fn clipboard_paste_files(&mut self) {
        if self.root_path.is_empty() {
            self.notice = Some((
                "Ctrl+V: kein Zielordner geöffnet".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        // Remote view → upload the clipboard's files into the current remote
        // folder via the backend (instead of a local std::fs copy).
        if let Some(rs) = &self.remote {
            let paths = match crate::shell_clipboard::read_files() {
                Some((p, _)) if !p.is_empty() => p,
                _ => {
                    self.notice = Some((
                        "Ctrl+V: Zwischenablage enthält keine Dateien".to_string(),
                        std::time::Instant::now(),
                    ));
                    return;
                }
            };
            self.start_remote_upload(paths, rs.backend.clone(), self.root_path.clone());
            return;
        }

        let dest = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));

        // Fast path: the clipboard still holds OUR filtered virtual files —
        // copy them directly without the COM stream round-trip.
        if let Some((seq, pairs)) = self.virtual_clip.clone() {
            if crate::virtual_clipboard::clipboard_sequence() == seq {
                self.notice = Some((
                    format!("📥 Einfügen (gefiltert): {} Datei(en)", pairs.len()),
                    std::time::Instant::now(),
                ));
                let (tx, rx) = unbounded();
                let h = crate::copy::start_copy_pairs(pairs, dest, Conflict::Rename, tx);
                self.copy_handle = Some(h);
                self.copy_rx = Some(rx);
                self.copy_progress = Some(CopyProgress {
                    files_done: 0,
                    files_total: 0,
                    bytes_done: 0,
                    bytes_total: 0,
                    elapsed_ms: 0,
                    current_path: String::new(),
                    errors: 0,
                    done: false,
                });
                self.copy_refresh_after = true;
                return;
            } else {
                self.virtual_clip = None;
            }
        }

        let (paths, is_cut) = match crate::shell_clipboard::read_files() {
            Some(v) => v,
            None => {
                self.notice = Some((
                    "Ctrl+V erkannt — aber Zwischenablage enthält keine Dateien".to_string(),
                    std::time::Instant::now(),
                ));
                return;
            }
        };
        if paths.is_empty() {
            self.notice = Some((
                "Ctrl+V erkannt — Zwischenablage enthält keine Dateien".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        self.notice = Some((
            format!(
                "📥 Füge {} {} ein…",
                paths.len(),
                if is_cut { "Datei(en) (verschieben)" } else { "Datei(en)" }
            ),
            std::time::Instant::now(),
        ));
        let common_parent = PathBuf::from(&paths[0])
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let opts = CopyOptions {
            root: common_parent,
            dest,
            preserve_structure: true,
            conflict: Conflict::Rename,
            mode: if is_cut { CopyMode::Move } else { CopyMode::Copy },
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let h = start_copy_from_paths(paths, opts, tx);
        self.copy_handle = Some(h);
        self.copy_rx = Some(rx);
        self.copy_progress = Some(CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            elapsed_ms: 0,
            current_path: String::new(),
            errors: 0,
            done: false,
        });
        self.copy_refresh_after = true;
    }

    #[cfg(not(windows))]
    pub(in crate::app) fn clipboard_copy_files(&mut self, _cut: bool) {}
    #[cfg(not(windows))]
    pub(in crate::app) fn clipboard_paste_files(&mut self) {}

    // ─── Drag-and-drop into the app ─────────────────────────────────────

    /// Copy (or move) OS paths into `dest`, on the copy worker. Conflicts
    /// auto-rename so a drop never overwrites. Shared by the OS drop handler.
    pub(in crate::app) fn copy_paths_into(&mut self, paths: Vec<String>, dest: PathBuf, move_files: bool) {
        if paths.is_empty() {
            return;
        }
        if self.copy_progress.as_ref().map(|p| !p.done).unwrap_or(false) {
            self.error_msg = Some("Es läuft bereits ein Kopiervorgang.".to_string());
            return;
        }
        let common_parent = PathBuf::from(&paths[0])
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let opts = CopyOptions {
            root: common_parent,
            dest,
            preserve_structure: true,
            conflict: Conflict::Rename,
            mode: if move_files { CopyMode::Move } else { CopyMode::Copy },
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let h = start_copy_from_paths(paths, opts, tx);
        self.copy_handle = Some(h);
        self.copy_rx = Some(rx);
        self.copy_progress = Some(CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            elapsed_ms: 0,
            current_path: String::new(),
            errors: 0,
            done: false,
        });
        self.copy_refresh_after = true;
    }

}
