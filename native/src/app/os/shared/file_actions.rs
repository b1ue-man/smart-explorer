use super::prelude::*;
use super::*;

impl App {
    /// The path the keyboard actions should act on: cursor first, else the
    /// first selected entry.
    pub(in crate::app) fn focus_path(&self) -> Option<String> {
        self.cursor.as_ref().map(|p| p.to_string()).or_else(|| {
            self.selection
                .iter()
                .next()
                .map(|k| sel_key_path(k).to_string())
        })
    }

    pub(in crate::app) fn show_properties(&mut self) {
        let p = match self.focus_path() {
            Some(p) => p,
            None => return,
        };
        show_properties_for_path(&p);
    }

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
        let Some(selected) = self.selection.iter().next() else {
            return;
        };
        let p = sel_key_path(selected).to_string();
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
            let spawn = std::thread::Builder::new()
                .name("remote-mkdir".into())
                .spawn(move || {
                    let result = (|| -> Result<String, String> {
                        let name = find_remote_unique_name(&*backend, &base, |index| {
                            if index == 1 {
                                "Neuer Ordner".to_string()
                            } else {
                                format!("Neuer Ordner ({index})")
                            }
                        })?;
                        let path = rjoin(&base, &name);
                        backend
                            .mkdir_all(&path)
                            .map_err(|error| format!("Ordner erstellen: {error}"))?;
                        Ok(format!("✓ Ordner erstellt: {name}"))
                    })();
                    let _ = tx.send(result);
                });
            match spawn {
                Ok(_) => {
                    self.remote_op_rx = Some(rx);
                    self.notice = Some((
                        "Ordner wird erstellt…".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    self.remote_op_rx = None;
                    self.error_msg = Some(format!(
                        "Remote-Ordnererstellung konnte nicht gestartet werden: {error}"
                    ));
                }
            }
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
                    format!(
                        "✓ Ordner erstellt: {}",
                        target
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default()
                    ),
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
            let spawn = std::thread::Builder::new()
                .name("remote-newfile".into())
                .spawn(move || {
                    use std::io::Write;
                    let result = (|| -> Result<String, String> {
                        let name = find_remote_unique_name(&*backend, &root, |index| {
                            if index == 1 {
                                format!("{base}.{ext}")
                            } else {
                                format!("{base} ({index}).{ext}")
                            }
                        })?;
                        let path = rjoin(&root, &name);
                        let staged = crate::vfs::unique_staging_path(&*backend, &path, "new-file")
                            .map_err(|error| error.to_string())?;
                        let result = (|| {
                            let mut writer = backend
                                .open_write(&staged)
                                .map_err(|error| error.to_string())?;
                            writer.flush().map_err(|error| error.to_string())?;
                            drop(writer);
                            backend
                                .rename_no_replace(&staged, &path)
                                .map_err(|error| error.to_string())
                        })();
                        if let Err(error) = result {
                            let _ = backend.remove_file(&staged);
                            return Err(error);
                        }
                        Ok(format!("✓ Datei erstellt: {name}"))
                    })();
                    let _ = tx.send(result.map_err(|error| format!("Datei erstellen: {error}")));
                });
            match spawn {
                Ok(_) => {
                    self.remote_op_rx = Some(rx);
                    self.notice = Some((
                        "Datei wird erstellt…".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    self.remote_op_rx = None;
                    self.error_msg = Some(format!(
                        "Remote-Dateierstellung konnte nicht gestartet werden: {error}"
                    ));
                }
            }
            return;
        }
        // Local view.
        let base_dir = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let created = (1..=10_000).find_map(|index| {
            let target = if index == 1 {
                base_dir.join(format!("{base}.{ext}"))
            } else {
                base_dir.join(format!("{base} ({index}).{ext}"))
            };
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(file) => Some(Ok((target, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        });
        match created.unwrap_or_else(|| {
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Kein freier Dateiname nach 10.000 Versuchen",
            ))
        }) {
            Ok((target, _file)) => {
                self.rescan();
                let nm = target
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.notice = Some((
                    format!("✓ Datei erstellt: {}", nm),
                    std::time::Instant::now(),
                ));
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
            let spawn = std::thread::Builder::new()
                .name("remote-rename".into())
                .spawn(move || {
                    let result = backend
                        .rename_no_replace(&old_fwd, &new_fwd)
                        .map(|_| format!("✓ Umbenannt: {}", draft))
                        .map_err(|error| format!("Ziel sicher umbenennen: {error}"));
                    let _ = tx.send(result);
                });
            match spawn {
                Ok(_) => self.remote_op_rx = Some(rx),
                Err(error) => {
                    self.remote_op_rx = None;
                    self.error_msg = Some(format!(
                        "Remote-Umbenennen konnte nicht gestartet werden: {error}"
                    ));
                }
            }
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
        let old_fwd = old.to_string_lossy().replace('\\', "/");
        let new_fwd = new.to_string_lossy().replace('\\', "/");
        let backend = crate::vfs::LocalBackend::new("/");
        match crate::vfs::Backend::rename_no_replace(&backend, &old_fwd, &new_fwd) {
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
        let mode = opts.mode;
        let filter = self.filter.clone();
        let root_prefix = self.root_prefix();
        self.start_copy_job(mode, false, move |tx| {
            start_copy_expanded(seeds, Some((filter, root_prefix)), opts, tx)
        });
    }

    // ─── Clipboard ──────────────────────────────────────────────────────
}
