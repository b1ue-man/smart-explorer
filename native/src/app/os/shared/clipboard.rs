use super::prelude::*;
use super::*;
use super::clipboard_lifecycle::{prepare_filtered_clipboard, PreparedTempClipboard};
use super::clipboard_state::PreparationResult;
use crate::app::shared_platform_helpers::ClipboardEffect;

impl App {
    pub(in crate::app) fn clipboard_copy_files(&mut self, cut: bool) {
        if !clipboard_file_ops_supported() {
            self.error_msg = Some("Datei-Zwischenablage ist auf dieser Plattform nicht verfügbar.".to_string());
            return;
        }
        if self.selection.is_empty() {
            self.notice = Some((
                "Nichts ausgewählt — bitte erst Dateien markieren".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        // Remote selection -> materialize files/folders in temp, then put those
        // local paths on the clipboard so they paste into Explorer or back here.
        if self.remote.is_some() && cut {
            self.cancel_clipboard_preparation();
            self.error_msg = Some("Remote-Ausschneiden wird nicht unterstützt. Bitte kopieren; die Quelldateien bleiben unverändert.".to_string());
            return;
        }
        if let Some(rs) = &self.remote {
            let items: Vec<(String, String, bool)> = self
                .entries
                .iter()
                .filter(|e| self.selection.contains(&e.key()))
                .map(|e| (e.path.to_string(), e.name.to_string(), e.is_dir))
                .collect();
            if items.is_empty() {
                self.notice = Some((
                    "Remote: nichts fuer die Zwischenablage ausgewaehlt.".to_string(),
                    std::time::Instant::now(),
                ));
                return;
            }
            let filter = (items.iter().any(|(_, _, is_dir)| *is_dir) && self.filter_is_active())
                .then(|| (self.filter.clone(), self.root_prefix()));
            let backend = rs.backend.clone();
            let Some(stamp) = self.begin_clipboard_preparation() else { return };
            let n = items.len();
            let (tx, rx) = unbounded();
            let spawn = std::thread::Builder::new()
                .name("clip-download".into())
                .spawn(move || {
                    let result = download_remote_clipboard_items(&*backend, &items, filter)
                        .map(PreparedTempClipboard::new);
                    // The owned result also cleans up if this receiver was replaced.
                    let _ = tx.send(PreparationResult { stamp, result });
                });
            match spawn {
                Ok(_) => {
                    self.clip_download_rx = Some(rx);
                    self.notice = Some((
                        format!("Bereite {} Element(e) fuer die Zwischenablage vor...", n),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    self.cancel_clipboard_preparation();
                    self.error_msg = Some(format!(
                        "Zwischenablage-Download konnte nicht gestartet werden: {error}"
                    ));
                }
            }
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
            let Some(stamp) = self.begin_clipboard_preparation() else { return };
            let (tx, rx) = unbounded();
            let spawn = std::thread::Builder::new()
                .name("clip-prepare".into())
                .spawn(move || {
                    let result = prepare_filtered_clipboard(seeds, filter, prefix);
                    let _ = tx.send(PreparationResult { stamp, result });
                });
            match spawn {
                Ok(_) => {
                    self.clip_prepare_rx = Some(rx);
                    self.notice = Some((
                        "Sammle gefilterte Dateien…".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    self.cancel_clipboard_preparation();
                    self.error_msg = Some(format!(
                        "Gefilterte Zwischenablage konnte nicht gestartet werden: {error}"
                    ));
                }
            }
            return;
        }

        // Plain CF_HDROP path (no filter, or cut, or files only).
        let paths: Vec<String> = self
            .selection
            .iter()
            .map(|k| sel_key_path(k).replace('/', "\\"))
            .collect();
        let effect = if cut {
            ClipboardEffect::Move
        } else {
            ClipboardEffect::Copy
        };
        self.cancel_clipboard_preparation();
        match write_clipboard_files(&paths, effect) {
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

    pub(in crate::app) fn clipboard_paste_files(&mut self) {
        if !clipboard_file_ops_supported() {
            self.error_msg = Some("Datei-Zwischenablage ist auf dieser Plattform nicht verfügbar.".to_string());
            return;
        }
        if self.clipboard_paste_is_pending() {
            return;
        }
        if self.root_path.is_empty() {
            self.notice = Some((
                "Ctrl+V: kein Zielordner geöffnet".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        // Resolve our virtual payload BEFORE choosing local copy or remote
        // upload: a descriptor/stream clipboard need not contain CF_HDROP.
        if let Some((seq, pairs)) = self.virtual_clip.clone() {
            if virtual_clipboard_sequence() == Some(seq) {
                if let Some(rs) = &self.remote {
                    self.start_filtered_remote_upload(pairs, rs.backend.clone(), self.root_path.clone());
                    return;
                }
                let dest = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
                let count = pairs.len();
                if self.start_copy_job(CopyMode::Copy, true, move |tx| {
                    crate::copy::start_copy_pairs(pairs, dest, Conflict::Rename, tx)
                }) {
                    self.notice = Some((
                        format!("📥 Einfügen (gefiltert): {} Datei(en)", count),
                        std::time::Instant::now(),
                    ));
                }
                return;
            } else {
                self.virtual_clip = None;
            }
        }

        let (paths, is_cut) = match read_clipboard_files() {
            Ok(Some(v)) => v,
            Ok(None) => {
                self.notice = Some((
                    "Ctrl+V erkannt — aber Zwischenablage enthält keine Dateien".to_string(),
                    std::time::Instant::now(),
                ));
                return;
            }
            Err(error) => {
                self.error_msg = Some(format!("Zwischenablage konnte nicht gelesen werden: {error}"));
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
        if let Some(rs) = &self.remote {
            if is_cut {
                self.error_msg = Some("Verschieben zu Remote wird nicht unterstützt. Bitte kopieren; die Quelldateien bleiben unverändert.".to_string());
                return;
            }
            self.start_remote_upload(paths, rs.backend.clone(), self.root_path.clone());
            return;
        }
        let dest = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let count = paths.len();
        let mode = if is_cut {
            CopyMode::Move
        } else {
            CopyMode::Copy
        };
        let common_parent = PathBuf::from(&paths[0])
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let opts = CopyOptions {
            root: common_parent,
            dest,
            preserve_structure: true,
            conflict: Conflict::Rename,
            mode,
        };
        if self.start_copy_job(mode, true, move |tx| start_copy_from_paths(paths, opts, tx)) {
            self.notice = Some((
                format!(
                    "📥 Füge {} {} ein…",
                    count,
                    if is_cut {
                        "Datei(en) (verschieben)"
                    } else {
                        "Datei(en)"
                    }
                ),
                std::time::Instant::now(),
            ));
        }
    }

    // ─── Drag-and-drop into the app ─────────────────────────────────────

    /// Copy (or move) OS paths into `dest`, on the copy worker. Conflicts
    /// auto-rename so a drop never overwrites. Shared by the OS drop handler.
    pub(in crate::app) fn copy_paths_into(
        &mut self,
        paths: Vec<String>,
        dest: PathBuf,
        move_files: bool,
    ) -> bool {
        if paths.is_empty() {
            return false;
        }
        let mode = if move_files {
            CopyMode::Move
        } else {
            CopyMode::Copy
        };
        let common_parent = PathBuf::from(&paths[0])
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let opts = CopyOptions {
            root: common_parent,
            dest,
            preserve_structure: true,
            conflict: Conflict::Rename,
            mode,
        };
        self.start_copy_job(mode, true, move |tx| start_copy_from_paths(paths, opts, tx))
    }
}
