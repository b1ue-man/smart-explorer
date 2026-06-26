use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn recompute_view(&mut self) {
        let prefix = self.root_prefix();
        let cf = CompiledFilter::compile(&self.filter);
        let key = self.sort_key;
        let dir = self.sort_dir;
        let dirs_first = self.dirs_first;
        self.summary_cache = None;
        self.sel_size_cache = (usize::MAX, usize::MAX, 0);
        self.view_dirty = false;

        // ─── Flat mode: contents of current dir only ──────────────────────
        if !self.recursive {
            let mut rows: Vec<(usize, u32)> = (0..self.entries.len())
                .filter(|&i| {
                    let e = &self.entries[i];
                    e.depth > 0 && cf.matches(e, &prefix)
                })
                .map(|i| (i, 0u32))
                .collect();
            let entries = &self.entries;
            rows.sort_unstable_by(|&(a, _), &(b, _)| {
                compare_entries(&entries[a], &entries[b], key, dir, dirs_first)
            });
            self.view = rows;
            self.last_view_recompute = Instant::now();
            return;
        }

        // ─── Tree mode: recursive view preserving folder structure ─────────
        let mut children_map: std::collections::HashMap<&str, Vec<usize>> =
            std::collections::HashMap::with_capacity(self.entries.len() / 4 + 16);
        for (i, e) in self.entries.iter().enumerate() {
            children_map.entry(e.parent.as_ref()).or_default().push(i);
        }

        let root_idx = match self
            .entries
            .iter()
            .position(|e| e.path.as_ref() == prefix.as_str())
        {
            Some(i) => i,
            None => {
                self.view = Vec::new();
                self.last_view_recompute = Instant::now();
                return;
            }
        };

        let mut file_matches = vec![false; self.entries.len()];
        for (i, e) in self.entries.iter().enumerate() {
            if !e.is_dir {
                file_matches[i] = cf.matches(e, &prefix);
            }
        }

        let mut has_match = vec![false; self.entries.len()];
        let mut stack: Vec<(usize, bool)> = vec![(root_idx, false)];
        while let Some((idx, expanded)) = stack.pop() {
            let e = &self.entries[idx];
            if !expanded {
                stack.push((idx, true));
                if let Some(children) = children_map.get(e.path.as_ref()) {
                    for &c in children {
                        if self.entries[c].is_dir {
                            stack.push((c, false));
                        }
                    }
                }
            } else {
                let mut any = false;
                if let Some(children) = children_map.get(e.path.as_ref()) {
                    for &c in children {
                        let ce = &self.entries[c];
                        if ce.is_dir {
                            if has_match[c] {
                                any = true;
                                break;
                            }
                        } else if file_matches[c] {
                            any = true;
                            break;
                        }
                    }
                }
                has_match[idx] = any;
            }
        }

        let dir_passes_view_filter = |idx: usize| -> bool {
            let e = &self.entries[idx];
            if !self.filter.include_dirs {
                return false;
            }
            if e.hidden && !self.filter.include_hidden {
                return false;
            }
            if e.system && !self.filter.include_system {
                return false;
            }
            true
        };

        let entries = &self.entries;
        let root_depth = entries[root_idx].depth;
        let mut visible: Vec<(usize, u32)> = Vec::new();

        struct Frame {
            children_remaining: std::vec::IntoIter<usize>,
        }
        let mut frames: Vec<Frame> = Vec::new();

        let make_sorted_children = |parent_idx: usize,
                                    children_map: &std::collections::HashMap<&str, Vec<usize>>,
                                    entries: &[FileEntry]|
         -> Vec<usize> {
            let parent_e = &entries[parent_idx];
            let mut out: Vec<usize> = match children_map.get(parent_e.path.as_ref()) {
                Some(v) => v.clone(),
                None => return Vec::new(),
            };
            out.retain(|&c| {
                let ce = &entries[c];
                if ce.is_dir {
                    has_match[c] && dir_passes_view_filter(c)
                } else {
                    file_matches[c]
                }
            });
            out.sort_unstable_by(|&a, &b| {
                compare_entries(&entries[a], &entries[b], key, dir, dirs_first)
            });
            out
        };

        frames.push(Frame {
            children_remaining: make_sorted_children(root_idx, &children_map, entries).into_iter(),
        });

        while let Some(frame) = frames.last_mut() {
            if let Some(idx) = frame.children_remaining.next() {
                let e = &entries[idx];
                let display_d = e.depth.saturating_sub(root_depth + 1);
                visible.push((idx, display_d));
                if e.is_dir {
                    let kids = make_sorted_children(idx, &children_map, entries);
                    frames.push(Frame {
                        children_remaining: kids.into_iter(),
                    });
                }
            } else {
                frames.pop();
            }
        }

        self.view = visible;
        self.last_view_recompute = Instant::now();
    }

    // ─── Selection / actions ────────────────────────────────────────────

    pub(in crate::app) fn select_all(&mut self) {
        self.selection = self
            .view
            .iter()
            .map(|&(i, _)| self.entries[i].key())
            .collect();
    }

    pub(in crate::app) fn copy_paths_to_clipboard(&self, ctx: &egui::Context) {
        let lines: Vec<String> = self
            .selection
            .iter()
            .map(|k| sel_key_path(k).replace('/', "\\"))
            .collect();
        ctx.copy_text(lines.join("\r\n"));
    }

    /// Move selection to the recycle bin on a background thread (a big
    /// selection can take seconds in the shell — that used to freeze the UI).
    pub(in crate::app) fn trash_selected(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        // Remote view → delete via the backend (SFTP/FTP/WebDAV unlink; Drive
        // moves to its trash). std::fs/the recycle bin can't touch remote paths.
        if let Some(rs) = &self.remote {
            let backend = rs.backend.clone();
            let items: Vec<(String, Option<String>, bool)> = self
                .entries
                .iter()
                .filter(|e| self.selection.contains(&e.key()))
                .map(|e| {
                    (
                        e.path.to_string(),
                        e.id.as_ref().map(|s| s.to_string()),
                        e.is_dir,
                    )
                })
                .collect();
            let removed: HashSet<Arc<str>> = self.selection.drain().collect();
            self.entries.retain(|e| !removed.contains(&e.key()));
            self.cursor = None;
            self.recompute_view();
            let (tx, rx) = unbounded();
            self.trash_rx = Some(rx);
            std::thread::Builder::new()
                .name("remote-delete".into())
                .spawn(move || {
                    fn remove_tree(be: &dyn crate::vfs::Backend, path: &str) -> Result<(), String> {
                        let entries = be.list_dir(path).map_err(|e| e.to_string())?;
                        for entry in entries {
                            let child = format!("{}/{}", path.trim_end_matches('/'), entry.name);
                            if entry.is_dir {
                                remove_tree(be, &child)?;
                            } else {
                                be.remove_file_id(&child, entry.id.as_deref())
                                    .map_err(|e| e.to_string())?;
                            }
                        }
                        be.remove_dir(path).map_err(|e| e.to_string())
                    }

                    let mut first_err: Option<String> = None;
                    for (p, id, is_dir) in &items {
                        let r = if *is_dir {
                            remove_tree(&*backend, p).map_err(std::io::Error::other)
                        } else {
                            backend.remove_file_id(p, id.as_deref())
                        };
                        if let Err(e) = r {
                            if first_err.is_none() {
                                first_err = Some(e.to_string());
                            }
                        }
                    }
                    let _ = tx.send(first_err);
                })
                .ok();
            return;
        }
        let paths: Vec<PathBuf> = self
            .selection
            .iter()
            .map(|k| PathBuf::from(sel_key_path(k).replace('/', std::path::MAIN_SEPARATOR_STR)))
            .collect();
        // Optimistic UI update; on failure drain_trash() rescans.
        let removed: HashSet<Arc<str>> = self.selection.drain().collect();
        self.entries.retain(|e| !removed.contains(&e.key()));
        self.cursor = None;
        self.recompute_view();

        let (tx, rx) = unbounded();
        self.trash_rx = Some(rx);
        std::thread::Builder::new()
            .name("trash".into())
            .spawn(move || {
                let res = trash::delete_all(&paths);
                let _ = tx.send(res.err().map(|e| e.to_string()));
            })
            .ok();
    }

    pub(in crate::app) fn open_in_explorer(&self, path: &str) {
        reveal_path_in_file_manager(path);
    }

    /// Open a file with its associated application.
    pub(in crate::app) fn open_path(&self, path: &str) {
        open_local_path(path, OpenMode::Default);
    }

    /// Show the native Windows "Open with…" chooser for a file (the `openas`
    /// shell verb). Remote files are downloaded to a temp copy first (see
    /// `open_file`), so this always runs on a real local path.
    pub(in crate::app) fn open_with_path(&self, path: &str) {
        open_local_path(path, OpenMode::With);
    }

    pub(in crate::app) fn launch_for_edit(
        &self,
        path: &str,
        mode: OpenMode,
    ) -> Option<EditProcess> {
        launch_local_for_edit(path, mode)
    }

    pub(in crate::app) fn open_selection(&mut self) {
        let targets: Vec<(String, String, bool, Option<String>)> = self
            .entries
            .iter()
            .filter(|e| self.selection.contains(&e.key()))
            .map(|e| {
                (
                    e.path.to_string(),
                    e.name.to_string(),
                    e.is_dir,
                    e.id.as_ref().map(|s| s.to_string()),
                )
            })
            .collect();
        if targets.len() == 1 && targets[0].2 {
            let p = PathBuf::from(targets[0].0.replace('/', std::path::MAIN_SEPARATOR_STR));
            self.start_scan(p);
            return;
        }
        for (p, name, _, id) in targets.into_iter().filter(|(_, _, d, _)| !*d).take(10) {
            self.open_file(p, name, id, OpenMode::Default);
        }
    }

    /// True when the selection is exactly one folder — the case where Enter /
    /// `open_selection` navigates into it instead of opening files.
    pub(in crate::app) fn selection_single_dir(&self) -> bool {
        let mut it = self
            .entries
            .iter()
            .filter(|e| self.selection.contains(&e.key()));
        match (it.next(), it.next()) {
            (Some(e), None) => e.is_dir,
            _ => false,
        }
    }
}
