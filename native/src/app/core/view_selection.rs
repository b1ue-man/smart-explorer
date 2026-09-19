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
            self.tree.rows = rows.clone();
            self.view = rows;
            self.reconcile_result_selection();
            self.last_view_recompute = Instant::now();
            return;
        }

        self.tree.rows =
            super::recursive_tree::result_rows(&self.entries, &prefix, &self.filter, |a, b| {
                compare_entries(&self.entries[a], &self.entries[b], key, dir, dirs_first)
            });
        self.view = self.tree.displayed(&self.entries);
        self.reconcile_result_selection();
        self.last_view_recompute = Instant::now();
    }

    fn reconcile_result_selection(&mut self) {
        if !self.selection.is_empty() {
            self.selection = self
                .tree
                .rows
                .iter()
                .map(|&(index, _)| self.entries[index].key())
                .filter(|key| self.selection.contains(key))
                .collect();
        }
    }

    // ─── Selection / actions ────────────────────────────────────────────

    pub(in crate::app) fn select_all(&mut self) {
        self.selection = self
            .tree
            .rows
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
