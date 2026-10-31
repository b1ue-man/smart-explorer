use super::prelude::*;
use super::*;
use std::time::Duration;

/// Cost of the last view rebuild in nanoseconds, stored by `recompute_view`.
static LAST_VIEW_REBUILD_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// A rebuild during a running scan waits this many times the cost of the last
/// one: rebuilds then take about a tenth of the time, whatever the size of the tree.
const VIEW_REBUILD_GAP_FACTOR: u32 = 10;
/// Floor of that gap, so that cheap rebuilds do not run on every frame.
const VIEW_REBUILD_MIN_GAP: Duration = Duration::from_millis(50);

/// The gap a rebuild of the view during a running scan waits after the last
/// rebuild, from the measured cost of that rebuild.
pub(in crate::app) fn view_rebuild_gap() -> Duration {
    view_rebuild_gap_for(Duration::from_nanos(
        LAST_VIEW_REBUILD_NANOS.load(std::sync::atomic::Ordering::Relaxed),
    ))
}

/// The gap that follows a rebuild which took `cost`.
pub(in crate::app) fn view_rebuild_gap_for(cost: Duration) -> Duration {
    VIEW_REBUILD_MIN_GAP.max(cost.saturating_mul(VIEW_REBUILD_GAP_FACTOR))
}

impl App {
    /// Rebuilds the view rows and records how long the rebuild took.
    pub(in crate::app) fn recompute_view(&mut self) {
        let started = Instant::now();
        self.rebuild_view_rows();
        let cost = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        LAST_VIEW_REBUILD_NANOS.store(cost, std::sync::atomic::Ordering::Relaxed);
    }

    fn rebuild_view_rows(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::{view_rebuild_gap_for, VIEW_REBUILD_MIN_GAP};
    use std::time::Duration;

    #[test]
    fn abgleich_task_view_rebuild_gap_follows_the_measured_cost() {
        assert_eq!(view_rebuild_gap_for(Duration::ZERO), VIEW_REBUILD_MIN_GAP);
        assert_eq!(
            view_rebuild_gap_for(Duration::from_millis(4)),
            Duration::from_millis(50)
        );
        assert_eq!(
            view_rebuild_gap_for(Duration::from_millis(40)),
            Duration::from_millis(400)
        );
    }
}
