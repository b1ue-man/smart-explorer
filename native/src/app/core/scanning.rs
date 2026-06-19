use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn start_scan(&mut self, root: PathBuf) {
        self.start_scan_navigated(root, true);
    }

    pub(in crate::app) fn start_scan_navigated(&mut self, root: PathBuf, record_history: bool) {
        if let Some(h) = self.scan_handle.take() {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        if record_history && !self.root_path.is_empty() {
            self.history.push(self.root_path.clone());
            self.forward.clear();
            if self.history.len() > 100 {
                self.history.remove(0);
            }
        }

        // Replace (not clear) to actually release the backing allocation.
        self.entries = Vec::new();
        self.view = Vec::new();
        self.selection = HashSet::new();
        self.last_anchor = None;
        self.cursor = None;
        self.progress = empty_progress();
        self.error_msg = None;
        self.failed_paths = Vec::new();
        self.summary_cache = None;
        self.view_dirty = false;
        self.band_press = None;
        self.band_active = false;
        // Opening a folder clears the NAME search so the new folder is fully
        // visible; other filters (type/size/date/ext) are kept on purpose.
        self.filter.text.clear();
        self.text_draft.clear();
        self.root_path = root.to_string_lossy().replace('\\', "/");

        let (tx, rx) = unbounded();
        let max_depth = if self.recursive { None } else { Some(1) };
        // Route remote roots through the backend walk; local roots (incl. drive
        // letters and UNC) keep the fast std::fs path. Decided centrally here by
        // path style, so every navigation entry point is handled without edits:
        // an active remote session stays remote as long as the target isn't a
        // local-style path; otherwise we drop back to local.
        let stay_remote = self.remote.is_some() && !is_local_style(&self.root_path);
        if !stay_remote {
            self.remote = None;
            // "Recent" is for local quick-access; remote locations live in the
            // saved-connections list instead (a remote path would fail a later
            // local scan).
            self.add_recent(&self.root_path.clone());
        }
        // Per-location sort preference (after `remote` is finalized, so the key
        // is namespaced by connection). Default if unset; no inheritance.
        self.dirs_first = self
            .dir_sort
            .get(&self.location_key(&self.root_path))
            .copied()
            .unwrap_or(DEFAULT_DIRS_FIRST);
        let handle = match self.remote.as_ref() {
            Some(rs) => crate::rscan::start_scan_backend(
                rs.backend.clone(),
                self.root_path.clone(),
                max_depth,
                tx,
            ),
            None => start_scan(root, false, max_depth, tx),
        };
        self.scan_rx = Some(rx);
        self.scan_handle = Some(handle);
        self.scan_running = true;
    }

    pub(in crate::app) fn navigate_up(&mut self) {
        if self.root_path.is_empty() {
            return;
        }
        let p = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = p.parent() {
            let parent_buf = parent.to_path_buf();
            if !parent_buf.as_os_str().is_empty() {
                self.start_scan(parent_buf);
            }
        }
    }

    pub(in crate::app) fn navigate_back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.forward.push(self.root_path.clone());
            if prev.is_empty() {
                self.show_landing_page();
                return;
            }
            let p = PathBuf::from(prev.replace('/', std::path::MAIN_SEPARATOR_STR));
            self.start_scan_navigated(p, false);
        }
    }

    pub(in crate::app) fn navigate_forward(&mut self) {
        if let Some(next) = self.forward.pop() {
            self.history.push(self.root_path.clone());
            if next.is_empty() {
                self.show_landing_page();
                return;
            }
            let p = PathBuf::from(next.replace('/', std::path::MAIN_SEPARATOR_STR));
            self.start_scan_navigated(p, false);
        }
    }

    pub(in crate::app) fn rescan(&mut self) {
        if self.root_path.is_empty() {
            return;
        }
        // Explicit refresh → drop the browsing cache so we re-list fresh.
        if let Some(rs) = &self.remote {
            rs.backend.invalidate_cache();
        }
        let p = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        self.start_scan_navigated(p, false);
    }

    pub(in crate::app) fn cancel_scan(&mut self) {
        if let Some(h) = self.scan_handle.take() {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.scan_running = false;
    }

    /// Run a recursive name search SERVER-SIDE (the SSH agent) under the current
    /// remote folder, replacing the listing with the streamed matches (shown as
    /// a flat list of paths relative to the search root). The server does the
    /// enumeration + name match, so huge remote trees are searchable without
    /// pulling the whole listing. RegExp isn't supported server-side.
    pub(in crate::app) fn run_remote_search(&mut self, query: String) {
        let backend = match self.remote.as_ref() {
            Some(rs) if rs.backend.supports_search() => rs.backend.clone(),
            _ => return,
        };
        if query.trim().is_empty() || self.root_path.is_empty() {
            return;
        }
        if self.filter.text_mode == TextMode::Regex {
            self.notice = Some((
                "Server-Suche unterstützt kein RegExp — bitte „enthält“ oder „Glob“.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        if let Some(h) = self.scan_handle.take() {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let spec = crate::agent_proto::SearchSpec {
            query: query.trim().to_string(),
            glob: self.filter.text_mode == TextMode::Glob,
            min_size: 0,
            max_size: 0,
            max_results: 100_000,
            want_dirs: self.filter.include_dirs,
        };
        // Reset the listing (mirrors start_scan_navigated).
        self.entries = Vec::new();
        self.view = Vec::new();
        self.selection = HashSet::new();
        self.last_anchor = None;
        self.cursor = None;
        self.progress = empty_progress();
        self.error_msg = None;
        self.failed_paths = Vec::new();
        self.summary_cache = None;
        self.view_dirty = false;
        // Flat, cross-folder results; the server already applied the name match,
        // so clear the name filter (size/date/ext filters still refine the view).
        self.recursive = false;
        self.filter.text.clear();
        self.text_draft.clear();
        let (tx, rx) = unbounded();
        let handle = crate::rscan::start_search_backend(backend, self.root_path.clone(), spec, tx);
        self.scan_rx = Some(rx);
        self.scan_handle = Some(handle);
        self.scan_running = true;
        self.notice = Some((
            "🔎 Server-Suche läuft…".to_string(),
            std::time::Instant::now(),
        ));
    }

    // ─── Folder index lifecycle ─────────────────────────────────────────
    pub(in crate::app) fn start_index_build(&mut self) {
        if self.index_building {
            return;
        }
        if let Some(c) = self.index_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.index_building = true;
        self.index_progress = 0;
        self.index_progress_path = String::new();
        let roots: Vec<PathBuf> = if self.drives.is_empty() {
            vec![self.home.clone()]
        } else {
            self.drives.iter().map(PathBuf::from).collect()
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        FolderIndex::build_async(roots, tx, cancel.clone());
        self.index_rx = Some(rx);
        self.index_cancel = Some(cancel);
    }

    pub(in crate::app) fn cancel_index_build(&mut self) {
        if let Some(c) = self.index_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.index_building = false;
        self.index_rx = None;
    }

    pub(in crate::app) fn drain_index(&mut self) {
        let rx = match self.index_rx.as_ref() {
            Some(r) => r,
            None => return,
        };
        for _ in 0..16 {
            match rx.try_recv() {
                Ok(IndexMsg::Progress { count, current }) => {
                    self.index_progress = count;
                    self.index_progress_path = current;
                }
                Ok(IndexMsg::Done(idx)) => {
                    let _ = idx.save(&folder_index_path());
                    self.folder_index = idx;
                    self.index_building = false;
                    self.index_rx = None;
                    self.index_cancel = None;
                    if !self.folder_search_query.is_empty() {
                        self.run_folder_search();
                    }
                    self.notice = Some((
                        format!("✓ Index gebaut: {} Ordner", self.folder_index.len()),
                        std::time::Instant::now(),
                    ));
                    break;
                }
                Err(_) => break,
            }
        }
    }

    /// Two-stage search: fuzzy scoring runs synchronously (pure CPU, fast),
    /// then a background thread stats the candidates and re-ranks by mtime —
    /// disk I/O never blocks the UI thread.
    pub(in crate::app) fn run_folder_search(&mut self) {
        if self.folder_search_query.is_empty() || self.folder_index.is_empty() {
            self.folder_search_results.clear();
            self.folder_search_rx = None;
            return;
        }
        let scored = self
            .folder_index
            .search_scored(&self.folder_search_query, 90);
        // Provisional, score-only results shown immediately
        self.folder_search_results = scored.iter().take(30).cloned().collect();
        self.folder_search_seq += 1;
        let seq = self.folder_search_seq;
        let (tx, rx) = unbounded();
        self.folder_search_rx = Some(rx);
        std::thread::Builder::new()
            .name("search-rank".into())
            .spawn(move || {
                let ranked = crate::folder_index::stat_and_rank(scored, 30);
                let _ = tx.send((seq, ranked));
            })
            .ok();
    }

    pub(in crate::app) fn drain_folder_search(&mut self) {
        let mut done = false;
        if let Some(rx) = self.folder_search_rx.as_ref() {
            while let Ok((seq, ranked)) = rx.try_recv() {
                if seq == self.folder_search_seq {
                    self.folder_search_results = ranked;
                    done = true;
                }
            }
        }
        if done {
            self.folder_search_rx = None;
        }
    }

    // ─── Background clipboard-key poller ────────────────────────────────
    // egui consumes Ctrl+C/X/V for its own text clipboard and, for a file
    // (CF_HDROP, no text) clipboard, emits NO paste event and triggers NO
    // repaint when idle — so update() never runs on the keypress and any
    // in-frame key poll is dead. A separate thread polls the real OS key
    // state ~30×/s, fires only when OUR window is the foreground window, and
    // wakes the UI via ctx.request_repaint().
}
