use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn save_recent(&self) {
        let txt = self.recent.join("\n");
        let _ = std::fs::write(settings_path(), txt);
    }

    pub(in crate::app) fn add_recent(&mut self, p: &str) {
        self.recent.retain(|x| x != p);
        self.recent.insert(0, p.to_string());
        self.recent.truncate(10);
        self.save_recent();
    }

    // ─── Favorites (starred folders) ────────────────────────────────────
    pub(in crate::app) fn save_favorites(&self) {
        let _ = std::fs::write(favorites_path(), self.favorites.join("\n"));
    }

    pub(in crate::app) fn is_favorite(&self, p: &str) -> bool {
        self.favorites.iter().any(|x| x == p)
    }

    /// Toggle a folder's starred state. Saves immediately — never deferred to
    /// on_exit (which clears state before any save could run).
    pub(in crate::app) fn toggle_favorite(&mut self, p: &str) {
        if let Some(i) = self.favorites.iter().position(|x| x == p) {
            self.favorites.remove(i);
            self.notice = Some(("☆ Aus Favoriten entfernt".to_string(), std::time::Instant::now()));
        } else {
            self.favorites.insert(0, p.to_string());
            self.notice = Some(("★ Zu Favoriten hinzugefügt".to_string(), std::time::Instant::now()));
        }
        self.save_favorites();
    }

    pub(in crate::app) fn save_ui_state(&self) {
        UiState {
            show_filters: self.show_filters,
            show_summary: self.show_summary,
        }
        .save();
    }

    pub(in crate::app) fn root_prefix(&self) -> String {
        self.root_path.replace('\\', "/").trim_end_matches('/').to_string()
    }

    /// A re-openable, connection-namespaced key for a location: a bare path
    /// locally, or `proto://user@host:port/path` on a remote — so favourites and
    /// per-folder prefs bind to the connection (the "link id"), not just a path.
    pub(in crate::app) fn location_key(&self, path: &str) -> String {
        let p = path.replace('\\', "/").trim_end_matches('/').to_string();
        match self.remote.as_ref().and_then(|rs| rs.endpoint_prefix.as_ref()) {
            Some(prefix) => format!("{}{}", prefix, p),
            None => p,
        }
    }

    /// Open a saved connection and navigate straight to `path` on it (used to
    /// re-open a remote favourite at its exact folder).
    pub(in crate::app) fn connect_saved_at(&mut self, c: &crate::creds::SavedConnection, path: &str) {
        let mut form = crate::connect::ConnectForm::from_saved(c);
        if !path.is_empty() {
            form.root = path.to_string();
        }
        let secret = crate::creds::get_secret(&c.account());
        crate::creds::touch_connection(&c.account());
        self.saved_connections = crate::creds::load_connections();
        self.begin_connect(form, secret);
    }

    /// Navigate to a favourite/location: a remote endpoint URL re-opens its
    /// connection at that path; a local path scans directly.
    pub(in crate::app) fn navigate_to_location(&mut self, loc: &str) {
        if crate::connect::is_remote_url(loc) {
            if let Some((c, path)) = crate::connect::saved_and_path(loc) {
                self.connect_saved_at(&c, &path);
            } else if loc.starts_with("gdrive://") {
                self.open_gdrive_browse(); // best-effort: Drive root
            } else {
                self.error_msg =
                    Some("Verbindung für diesen Favoriten nicht gefunden — zuerst verbinden".into());
            }
        } else {
            self.start_scan(PathBuf::from(loc.replace('/', std::path::MAIN_SEPARATOR_STR)));
        }
    }

    pub(in crate::app) fn filter_is_active(&self) -> bool {
        let f = &self.filter;
        !f.text.is_empty()
            || !f.extensions.is_empty()
            || f.size.min.is_some()
            || f.size.max.is_some()
            || f.mtime.min.is_some()
            || f.mtime.max.is_some()
            || f.btime.min.is_some()
            || f.btime.max.is_some()
            || !f.include_files
            || !f.include_dirs
            || !f.include_hidden
            || !f.include_system
    }

    // ─── Tabs ────────────────────────────────────────────────────────────

    /// Exchange the App's working fields with the state parked in `tabs[i]`.
    pub(in crate::app) fn swap_with_tab(&mut self, i: usize) {
        let mut t = std::mem::take(&mut self.tabs[i]);
        std::mem::swap(&mut t.root_path, &mut self.root_path);
        std::mem::swap(&mut t.entries, &mut self.entries);
        std::mem::swap(&mut t.view, &mut self.view);
        std::mem::swap(&mut t.selection, &mut self.selection);
        std::mem::swap(&mut t.last_anchor, &mut self.last_anchor);
        std::mem::swap(&mut t.cursor, &mut self.cursor);
        std::mem::swap(&mut t.scan_rx, &mut self.scan_rx);
        std::mem::swap(&mut t.scan_handle, &mut self.scan_handle);
        std::mem::swap(&mut t.progress, &mut self.progress);
        std::mem::swap(&mut t.scan_running, &mut self.scan_running);
        std::mem::swap(&mut t.history, &mut self.history);
        std::mem::swap(&mut t.forward, &mut self.forward);
        std::mem::swap(&mut t.failed_paths, &mut self.failed_paths);
        std::mem::swap(&mut t.view_dirty, &mut self.view_dirty);
        std::mem::swap(&mut t.remote, &mut self.remote);
        std::mem::swap(&mut t.net_conn, &mut self.net_conn);
        std::mem::swap(&mut t.filter, &mut self.filter);
        std::mem::swap(&mut t.sort_key, &mut self.sort_key);
        std::mem::swap(&mut t.sort_dir, &mut self.sort_dir);
        std::mem::swap(&mut t.text_draft, &mut self.text_draft);
        std::mem::swap(&mut t.ext_draft, &mut self.ext_draft);
        std::mem::swap(&mut t.size_min_draft, &mut self.size_min_draft);
        std::mem::swap(&mut t.size_max_draft, &mut self.size_max_draft);
        std::mem::swap(&mut t.filter_pending_at, &mut self.filter_pending_at);
        std::mem::swap(&mut t.mtime_min_date, &mut self.mtime_min_date);
        std::mem::swap(&mut t.mtime_max_date, &mut self.mtime_max_date);
        std::mem::swap(&mut t.btime_min_date, &mut self.btime_min_date);
        std::mem::swap(&mut t.btime_max_date, &mut self.btime_max_date);
        self.tabs[i] = t;
        // dirs_first is per-location (not parked in the tab) — re-derive it for
        // whatever path is now active so the toggle + next sort match.
        self.dirs_first = self
            .dir_sort
            .get(&self.location_key(&self.root_path))
            .copied()
            .unwrap_or(DEFAULT_DIRS_FIRST);
    }

    pub(in crate::app) fn switch_tab(&mut self, to: usize) {
        if to == self.active_tab || to >= self.tabs.len() {
            return;
        }
        let from = self.active_tab;
        self.swap_with_tab(from);
        self.swap_with_tab(to);
        self.active_tab = to;
        // Switching tabs ends any in-progress filter-driven navigation.
        self.search_nav_from_filter = false;
        // In split mode, a tab selection lands in whichever pane has focus, so
        // the user can re-target the right pane (not always the left).
        if self.split {
            self.panes[self.focused_pane.min(1)] = to;
        }
        self.band_press = None;
        self.band_active = false;
        self.summary_cache = None;        self.sel_size_cache = (usize::MAX, usize::MAX, 0);
        if self.view_dirty {
            self.recompute_view();
        }
    }

    /// Toggle split-screen (two tabs side by side). Enabling guarantees a
    /// second tab exists (cloning the current location) without moving focus.
    pub(in crate::app) fn toggle_split(&mut self) {
        if self.split {
            self.split = false;
            return;
        }
        if self.tabs.len() < 2 {
            let cur = self.root_path.clone();
            self.tabs.push(TabState::default());
            let new_idx = self.tabs.len() - 1;
            let prev = self.active_tab;
            self.switch_tab(new_idx);
            let target = if cur.is_empty() {
                self.home.clone()
            } else {
                PathBuf::from(cur.replace('/', std::path::MAIN_SEPARATOR_STR))
            };
            self.start_scan_navigated(target, false);
            self.switch_tab(prev);
        }
        let other = (0..self.tabs.len())
            .find(|&i| i != self.active_tab)
            .unwrap_or(self.active_tab);
        self.panes = [self.active_tab, other];
        self.focused_pane = 0;
        self.split = true;
    }

}
