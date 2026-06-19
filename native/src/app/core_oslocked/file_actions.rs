use super::prelude::*;
use super::*;

impl App {
    /// The path the keyboard actions should act on: cursor first, else the
    /// first selected entry.
    pub(in crate::app) fn focus_path(&self) -> Option<String> {
        self.cursor
            .as_ref()
            .map(|p| p.to_string())
            .or_else(|| self.selection.iter().next().map(|k| sel_key_path(k).to_string()))
    }

    /// Open the native file Properties sheet for the focused item.
    #[cfg(windows)]
    pub(in crate::app) fn show_properties(&mut self) {
        let p = match self.focus_path() {
            Some(p) => p.replace('/', "\\"),
            None => return,
        };
        use windows_sys::Win32::UI::Shell::{
            ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
        };
        let verb: Vec<u16> = "properties".encode_utf16().chain(Some(0)).collect();
        let file: Vec<u16> = p.encode_utf16().chain(Some(0)).collect();
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_INVOKEIDLIST;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.nShow = 1; // SW_SHOWNORMAL
        unsafe {
            ShellExecuteExW(&mut info);
        }
    }

    #[cfg(not(windows))]
    pub(in crate::app) fn show_properties(&mut self) {}

    /// Invert the selection within the current view.
    pub(in crate::app) fn invert_selection(&mut self) {
        let mut new: HashSet<Arc<str>> = HashSet::new();
        for &(i, _) in &self.view {
            let k = self.entries[i].key();
            if !self.selection.contains(&k) {
                new.insert(k);
            }
        }
        self.selection = new;
        self.cursor = None;
    }

    /// Permanently delete the selection (bypassing the recycle bin), after an
    /// explicit confirmation. Runs the deletes on a worker thread.
    pub(in crate::app) fn delete_permanent(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let n = self.selection.len();
        if !confirm_yes_no(
            "Endgültig löschen",
            &format!(
                "{} Eintrag/Einträge UNWIDERRUFLICH löschen (nicht in den Papierkorb)?",
                n
            ),
        ) {
            return;
        }
        let paths: Vec<PathBuf> = self
            .selection
            .iter()
            .map(|k| PathBuf::from(sel_key_path(k).replace('/', std::path::MAIN_SEPARATOR_STR)))
            .collect();
        let removed: HashSet<Arc<str>> = self.selection.drain().collect();
        self.entries.retain(|e| !removed.contains(&e.key()));
        self.cursor = None;
        self.recompute_view();

        let (tx, rx) = unbounded();
        self.trash_rx = Some(rx); // reuse the trash result channel/drain
        std::thread::Builder::new()
            .name("delete-permanent".into())
            .spawn(move || {
                let mut first_err: Option<String> = None;
                for p in &paths {
                    let res = if p.is_dir() {
                        std::fs::remove_dir_all(p)
                    } else {
                        std::fs::remove_file(p)
                    };
                    if let Err(e) = res {
                        if first_err.is_none() {
                            first_err = Some(e.to_string());
                        }
                    }
                }
                let _ = tx.send(first_err);
            })
            .ok();
    }

    pub(in crate::app) fn star_current_folder(&mut self) {
        if self.root_path.is_empty() {
            return;
        }
        let key = self.location_key(&self.root_path);
        self.toggle_favorite(&key);
    }

    pub(in crate::app) fn open_rename(&mut self) {
        if self.selection.len() != 1 {
            self.notice = Some((
                "Zum Umbenennen genau einen Eintrag auswählen".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let p = sel_key_path(self.selection.iter().next().unwrap()).to_string();
        let name = p.rsplit('/').next().unwrap_or("").to_string();
        self.rename_open = Some((p, name));
        self.rename_focus = true;
    }

    pub(in crate::app) fn create_new_folder(&mut self) {
        if self.root_path.is_empty() {
            return;
        }
        // Remote view → create via the backend (off the UI thread).
        if let Some(rs) = &self.remote {
            if self.remote_op_rx.is_some() {
                return;
            }
            let backend = rs.backend.clone();
            let base = self.root_path.trim_end_matches('/').to_string();
            let (tx, rx) = unbounded();
            self.remote_op_rx = Some(rx);
            std::thread::Builder::new()
                .name("remote-mkdir".into())
                .spawn(move || {
                    let mut name = "Neuer Ordner".to_string();
                    let mut i = 2;
                    while backend.exists(&format!("{}/{}", base, name)) && i < 1000 {
                        name = format!("Neuer Ordner ({})", i);
                        i += 1;
                    }
                    let path = format!("{}/{}", base, name);
                    let _ = tx.send(
                        backend
                            .mkdir_all(&path)
                            .map(|_| format!("✓ Ordner erstellt: {}", name))
                            .map_err(|e| format!("Ordner erstellen: {}", e)),
                    );
                })
                .ok();
            self.notice = Some(("Ordner wird erstellt…".to_string(), std::time::Instant::now()));
            return;
        }
        let base = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let mut target = base.join("Neuer Ordner");
        let mut i = 2;
        while target.exists() {
            target = base.join(format!("Neuer Ordner ({})", i));
            i += 1;
        }
        match std::fs::create_dir(&target) {
            Ok(_) => {
                self.rescan();
                self.notice = Some((
                    format!("✓ Ordner erstellt: {}", target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => self.error_msg = Some(format!("Ordner erstellen: {}", e)),
        }
    }

    /// Create a new empty editable file (`base.ext`) in the current folder, with
    /// a unique name. Local: created + opened for editing. Remote: created via
    /// the backend off-thread (open it afterwards by double-click).
    pub(in crate::app) fn create_new_file(&mut self, base: &str, ext: &str) {
        if self.root_path.is_empty() {
            return;
        }
        // Remote view → create via the backend (threaded).
        if let Some(rs) = &self.remote {
            if self.remote_op_rx.is_some() {
                return;
            }
            let backend = rs.backend.clone();
            let root = self.root_path.trim_end_matches('/').to_string();
            let (base, ext) = (base.to_string(), ext.to_string());
            let (tx, rx) = unbounded();
            self.remote_op_rx = Some(rx);
            std::thread::Builder::new()
                .name("remote-newfile".into())
                .spawn(move || {
                    use std::io::Write;
                    let mut name = format!("{}.{}", base, ext);
                    let mut i = 2;
                    while backend.exists(&format!("{}/{}", root, name)) && i < 1000 {
                        name = format!("{} ({}).{}", base, i, ext);
                        i += 1;
                    }
                    let path = format!("{}/{}", root, name);
                    let r = (|| -> Result<(), String> {
                        let mut w = backend.open_write(&path).map_err(|e| e.to_string())?;
                        w.flush().map_err(|e| e.to_string())?;
                        Ok(())
                    })();
                    let _ = tx.send(
                        r.map(|_| format!("✓ Datei erstellt: {}", name))
                            .map_err(|e| format!("Datei erstellen: {}", e)),
                    );
                })
                .ok();
            self.notice = Some(("Datei wird erstellt…".to_string(), std::time::Instant::now()));
            return;
        }
        // Local view.
        let base_dir = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let mut target = base_dir.join(format!("{}.{}", base, ext));
        let mut i = 2;
        while target.exists() {
            target = base_dir.join(format!("{} ({}).{}", base, i, ext));
            i += 1;
        }
        match std::fs::File::create(&target) {
            Ok(_) => {
                self.rescan();
                let nm = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.notice = Some((format!("✓ Datei erstellt: {}", nm), std::time::Instant::now()));
                self.open_path(&target.to_string_lossy().replace('\\', "/"));
            }
            Err(e) => self.error_msg = Some(format!("Datei erstellen: {}", e)),
        }
    }

    pub(in crate::app) fn move_cursor_to(&mut self, pos: usize, shift: bool) {
        if self.view.is_empty() {
            return;
        }
        let pos = pos.min(self.view.len() - 1);
        let path = self.entries[self.view[pos].0].path.clone();
        let key = self.entries[self.view[pos].0].key();
        if shift {
            if let Some(anchor) = self.last_anchor.clone() {
                if let Some(a) = self
                    .view
                    .iter()
                    .position(|&(i, _)| self.entries[i].key() == anchor)
                {
                    let (lo, hi) = if a < pos { (a, pos) } else { (pos, a) };
                    self.selection.clear();
                    for r in lo..=hi {
                        self.selection.insert(self.entries[self.view[r].0].key());
                    }
                } else {
                    self.selection.clear();
                    self.selection.insert(key.clone());
                    self.last_anchor = Some(key.clone());
                }
            } else {
                self.selection.clear();
                self.selection.insert(key.clone());
                self.last_anchor = Some(key.clone());
            }
        } else {
            self.selection.clear();
            self.selection.insert(key.clone());
            self.last_anchor = Some(key.clone());
        }
        self.cursor = Some(path);
        self.pending_scroll_row = Some(pos);
    }

    pub(in crate::app) fn cursor_pos_in_view(&self) -> Option<usize> {
        let c = self.cursor.as_ref()?;
        self.view
            .iter()
            .position(|&(i, _)| self.entries[i].path == *c)
    }

    pub(in crate::app) fn move_cursor(&mut self, delta: isize, shift: bool) {
        if self.view.is_empty() {
            return;
        }
        let next = match self.cursor_pos_in_view() {
            Some(c) => (c as isize + delta).clamp(0, self.view.len() as isize - 1) as usize,
            None => {
                if delta >= 0 {
                    0
                } else {
                    self.view.len() - 1
                }
            }
        };
        self.move_cursor_to(next, shift);
    }

    pub(in crate::app) fn type_to_jump(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.type_jump_at.elapsed().as_millis() > 800 {
            self.type_jump.clear();
        }
        self.type_jump.push_str(&text.to_lowercase());
        self.type_jump_at = Instant::now();
        let needle = self.type_jump.clone();
        if let Some(pos) = self
            .view
            .iter()
            .position(|&(i, _)| self.entries[i].name.to_lowercase().starts_with(&needle))
        {
            self.move_cursor_to(pos, false);
        }
    }

    pub(in crate::app) fn confirm_rename(&mut self) {
        let (path, draft) = match self.rename_open.take() {
            Some(v) => v,
            None => return,
        };
        let draft = draft.trim().to_string();
        if draft.is_empty() {
            return;
        }
        // Remote view → rename via the backend (off the UI thread).
        if let Some(rs) = &self.remote {
            if draft.contains('/') || draft.contains('\\') {
                self.error_msg = Some("Name darf keine Schrägstriche enthalten.".to_string());
                return;
            }
            let old_fwd = path.clone();
            let parent = old_fwd.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            let new_fwd = if parent.is_empty() {
                draft.clone()
            } else {
                format!("{}/{}", parent, draft)
            };
            if new_fwd == old_fwd || self.remote_op_rx.is_some() {
                return;
            }
            let backend = rs.backend.clone();
            let (tx, rx) = unbounded();
            self.remote_op_rx = Some(rx);
            std::thread::Builder::new()
                .name("remote-rename".into())
                .spawn(move || {
                    let _ = tx.send(
                        backend
                            .rename(&old_fwd, &new_fwd)
                            .map(|_| format!("✓ Umbenannt: {}", draft))
                            .map_err(|e| format!("Umbenennen: {}", e)),
                    );
                })
                .ok();
            self.selection.clear();
            return;
        }
        let old = PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let new = match old.parent() {
            Some(p) => p.join(&draft),
            None => return,
        };
        if new == old {
            return;
        }
        if new.exists() {
            self.error_msg = Some(format!("Ziel existiert bereits: {}", draft));
            return;
        }
        match std::fs::rename(&old, &new) {
            Ok(_) => {
                self.selection.clear();
                self.rescan();
            }
            Err(e) => self.error_msg = Some(format!("Umbenennen: {}", e)),
        }
    }

    pub(in crate::app) fn confirm_copy(&mut self) {
        // Selection seeds; the worker thread expands directories recursively
        // and applies the current filter (no UI freeze on big subtrees).
        let seeds: Vec<FileEntry> = self
            .entries
            .iter()
            .filter(|e| self.selection.contains(&e.key()))
            .cloned()
            .collect();
        if seeds.is_empty() || self.copy_dest.is_empty() {
            return;
        }
        let opts = CopyOptions {
            root: PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR)),
            dest: PathBuf::from(&self.copy_dest),
            preserve_structure: self.copy_preserve,
            conflict: self.copy_conflict,
            mode: self.copy_mode_pending,
        };
        let (tx, rx) = unbounded();
        let h = start_copy_expanded(
            seeds,
            Some((self.filter.clone(), self.root_prefix())),
            opts,
            tx,
        );
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
        self.copy_errors.clear();
    }

    // ─── Clipboard ──────────────────────────────────────────────────────

}
