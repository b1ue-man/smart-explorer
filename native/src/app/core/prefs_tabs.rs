use super::prelude::*;
use super::*;

impl App {
    pub(crate) fn configure_appearance(&self, ctx: &egui::Context) {
        theme::install(ctx, self.appearance.mode);
    }

    fn save_recent(recent: &[String]) -> std::io::Result<()> {
        std::fs::write(settings_path(), recent.join("\n"))
    }

    pub(in crate::app) fn add_recent(&mut self, p: &str) {
        let mut next = self.recent.clone();
        next.retain(|x| x != p);
        next.insert(0, p.to_string());
        next.truncate(10);
        match Self::save_recent(&next) {
            Ok(()) => self.recent = next,
            Err(error) => {
                self.error_msg = Some(format!("Zuletzt verwendet speichern: {error}"));
            }
        }
    }

    // ─── Favorites (starred folders) ────────────────────────────────────
    pub(in crate::app) fn is_favorite(&self, p: &str) -> bool {
        self.favorites.iter().any(|x| x == p)
    }

    /// Toggle a folder's starred state. Saves immediately — never deferred to
    /// on_exit (which clears state before any save could run).
    pub(in crate::app) fn toggle_favorite(&mut self, p: &str) {
        let mut next = self.favorites.clone();
        let notice = if let Some(i) = next.iter().position(|x| x == p) {
            next.remove(i);
            "☆ Aus Favoriten entfernt"
        } else {
            next.insert(0, p.to_string());
            "★ Zu Favoriten hinzugefügt"
        };
        match crate::connect::save_favorites(&next) {
            Ok(()) => {
                self.favorites = next;
                self.notice = Some((notice.to_string(), std::time::Instant::now()));
            }
            Err(error) => {
                self.error_msg = Some(format!("Favoriten speichern: {error}"));
            }
        }
    }

    pub(in crate::app) fn save_ui_state(&mut self) {
        if let Err(error) = (UiState {
            show_filters: self.show_filters,
            show_summary: self.show_summary,
            appearance: self.appearance,
        })
        .save()
        {
            self.error_msg = Some(format!("Oberflächenzustand speichern: {error}"));
        }
    }

    pub(in crate::app) fn root_prefix(&self) -> String {
        // The scanner's normalized spelling also owns descendant paths (for
        // example an uppercase drive letter and stripped verbatim prefix).
        self.entries
            .iter()
            .find(|entry| entry.depth == 0)
            .map(|entry| entry.path.to_string())
            .unwrap_or_else(|| {
                if self.remote.is_none() || is_local_style(&self.root_path) {
                    crate::local_access::display_path(&crate::local_access::normalize_scan_root(
                        Path::new(&self.root_path),
                    ))
                    .replace('\\', "/")
                } else {
                    self.root_path.replace('\\', "/")
                }
            })
    }

    /// A re-openable, connection-namespaced key for a location: a bare path
    /// locally, or `proto://user@host:port/path` on a remote — so favourites and
    /// per-folder prefs bind to the connection (the "link id"), not just a path.
    pub(in crate::app) fn location_key(&self, path: &str) -> String {
        crate::connect::location_key(
            self.remote
                .as_ref()
                .and_then(|rs| rs.endpoint_prefix.as_deref()),
            path,
        )
    }

    /// Open a saved connection and navigate straight to `path` on it (used to
    /// re-open a remote favourite at its exact folder).
    pub(in crate::app) fn connect_saved_at(
        &mut self,
        c: &crate::creds::SavedConnection,
        path: &str,
    ) {
        let mut form = crate::connect::ConnectForm::from_saved(c);
        if !path.is_empty() {
            form.root = path.to_string();
        }
        let secret = crate::creds::get_secret(&c.account());
        let touch_error = crate::creds::touch_connection(&c.account())
            .err()
            .map(|error| format!("Verbindungsliste konnte nicht aktualisiert werden: {error}"));
        self.saved_connections = crate::creds::load_connections();
        self.begin_connect(form, secret);
        if let Some(detail) = touch_error {
            self.push_app_error("Gespeicherte Verbindung", detail.clone());
            if self.error_msg.is_none() {
                self.error_msg = Some(detail);
            }
        }
    }

    /// Navigate to a favourite/location: a remote endpoint URL re-opens its
    /// connection at that path; a local path scans directly.
    pub(in crate::app) fn navigate_to_location(&mut self, loc: &str) {
        // Share peers are ID-addressed (`share://direct/<id>/path`): they open
        // through the Share worker, never through a saved-connection lookup.
        if let Some((target, path)) = crate::share::PeerOpenTarget::from_endpoint(loc) {
            self.open_share_target_at(target, Some(path));
            return;
        }
        if crate::connect::is_remote_url(loc) {
            if let Some((c, path)) = crate::connect::saved_and_path(loc) {
                self.connect_saved_at(&c, &path);
            } else if loc.starts_with("gdrive://") {
                self.open_gdrive_browse(); // best-effort: Drive root
            } else {
                self.error_msg = Some(
                    "Verbindung für diesen Favoriten nicht gefunden — zuerst verbinden".into(),
                );
            }
        } else {
            self.start_scan(PathBuf::from(
                loc.replace('/', std::path::MAIN_SEPARATOR_STR),
            ));
        }
    }

    pub(in crate::app) fn filter_is_active(&self) -> bool {
        crate::filter::filter_prunes(&self.filter)
    }

    // ─── Tabs ────────────────────────────────────────────────────────────

    /// Exchange the App's working fields with the state parked in `tabs[i]`.
    pub(in crate::app) fn swap_with_tab(&mut self, i: usize) {
        let mut t = std::mem::take(&mut self.tabs[i]);
        std::mem::swap(&mut t.root_path, &mut self.root_path);
        std::mem::swap(&mut t.recursive, &mut self.recursive);
        std::mem::swap(&mut t.entries, &mut self.entries);
        std::mem::swap(&mut t.view, &mut self.view);
        std::mem::swap(&mut t.tree, &mut self.tree);
        std::mem::swap(&mut t.read_access, &mut self.read_access);
        std::mem::swap(&mut t.selection, &mut self.selection);
        std::mem::swap(&mut t.last_anchor, &mut self.last_anchor);
        std::mem::swap(&mut t.cursor, &mut self.cursor);
        std::mem::swap(&mut t.scan_rx, &mut self.scan_rx);
        std::mem::swap(&mut t.scan_handle, &mut self.scan_handle);
        std::mem::swap(&mut t.progress, &mut self.progress);
        std::mem::swap(&mut t.scan_running, &mut self.scan_running);
        std::mem::swap(&mut t.scan_was_canceled, &mut self.scan_was_canceled);
        std::mem::swap(&mut t.scan_retention, &mut self.scan_retention);
        std::mem::swap(&mut t.scan_truncated, &mut self.scan_truncated);
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
        self.summary_cache = None;
        self.sel_size_cache = (usize::MAX, usize::MAX, 0);
        if self.view_dirty {
            if self.scan_truncated && !self.scan_was_canceled {
                self.filter_changed();
            } else {
                self.recompute_view();
            }
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
