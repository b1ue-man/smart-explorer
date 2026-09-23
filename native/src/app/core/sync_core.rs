use super::prelude::*;
use super::*;

impl App {
    /// One-way mirror using the selected folder's live backend and root.
    pub(in crate::app) fn start_mirror(&mut self, dst: crate::vfs::BackendHandle, destination: String) {
        if self.root_path.is_empty() || self.sync_running || self.bisync_running
            || self.job_connect_rx.is_some() {
            return;
        }
        let (src, root) = self.pane_backend(self.active_tab);
        let dst = crate::vfs::sync_backend(dst);
        let (tx, rx) = unbounded();
        let h = crate::sync::start_sync(
            src,
            root,
            dst,
            destination,
            crate::sync::SyncOptions {
                delete_extra: false,
                dry_run: false,
            },
            tx,
        );
        self.sync_cancel = Some(h.cancel);
        self.sync_rx = Some(rx);
        self.sync_running = true;
        self.notice = Some((
            "⇅ Spiegelung gestartet…".to_string(),
            std::time::Instant::now(),
        ));
    }

    pub(in crate::app) fn drain_sync(&mut self) {
        let msg = match self.sync_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(message)) => message,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.sync_rx = None;
                self.sync_running = false;
                self.sync_progress = None;
                self.sync_cancel = None;
                self.error_msg =
                    Some("Spiegelungs-Thread wurde ohne Ergebnis beendet.".to_string());
                return;
            }
        };
        match msg {
            crate::sync::SyncMsg::Progress(p) => {
                self.sync_progress = Some(p);
            }
            crate::sync::SyncMsg::Done(r) => {
                let canceled = self
                    .sync_cancel
                    .as_ref()
                    .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Relaxed));
                self.sync_rx = None;
                self.sync_running = false;
                self.sync_progress = None;
                self.sync_cancel = None;
                let omitted = r.omissions.summary().map(|s| format!("; {s}")).unwrap_or_default();
                if r.stats.errors > 0 {
                    let example = r
                        .errors
                        .first()
                        .map(|(path, detail)| format!(" ({path}: {detail})"))
                        .unwrap_or_default();
                    self.error_msg = Some(format!(
                        "Spiegelung unvollständig: {} kopiert, {} Fehler{}{omitted}",
                        r.stats.copied, r.stats.errors, example
                    ));
                } else if canceled {
                    self.notice = Some((
                        format!("Spiegelung abgebrochen: {} bereits kopiert{omitted}", r.stats.copied),
                        std::time::Instant::now(),
                    ));
                } else if let Some(omitted) = r.omissions.summary() {
                    self.notice = Some((
                        format!("⚠ Spiegelung mit Auslassungen: {} kopiert; {omitted}", r.stats.copied),
                        std::time::Instant::now(),
                    ));
                } else {
                    self.notice = Some((
                        format!(
                            "✓ Spiegelung fertig: {} kopiert, {} übersprungen ({} MB)",
                            r.stats.copied,
                            r.stats.skipped,
                            r.stats.bytes / 1_048_576
                        ),
                        std::time::Instant::now(),
                    ));
                }
            }
        }
    }

    /// Two-way sync the current location with safe, reversible defaults.
    pub(in crate::app) fn start_bisync(&mut self, b: crate::vfs::BackendHandle, destination: String) {
        if self.root_path.is_empty() {
            return;
        }
        let (a, root_a) = self.pane_backend(self.active_tab);
        self.launch_bisync(
            a,
            root_a,
            b,
            destination,
            crate::bisync::BisyncOptions::default(),
            true,
            Vec::new(),
            (0, 0, 0, 0),
            None,
        );
    }

    /// Shared checked launcher for ad-hoc, saved-job, and split-view syncs.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app) fn launch_bisync(
        &mut self,
        a: crate::vfs::BackendHandle,
        root_a: String,
        b: crate::vfs::BackendHandle,
        root_b: String,
        opts: crate::bisync::BisyncOptions,
        include_hidden: bool,
        ignore: Vec<String>,
        bounds: (u64, u64, i64, i64),
        job_id: Option<String>,
    ) {
        if self.bisync_running
            || self.sync_running
            || self.conflict_resolution.is_some()
            || self.merge.is_some()
            || self.merge_load_rx.is_some()
            || self.merge_apply_rx.is_some()
            || self.conflict_baseline_dirty
        {
            self.notice = Some((
                "Es läuft bereits ein Sync oder eine Konfliktauflösung — bitte warten.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let mut glob_builder = globset::GlobSetBuilder::new();
        for pattern in ignore.iter().map(|pattern| pattern.trim()) {
            if pattern.is_empty() {
                continue;
            }
            match globset::Glob::new(pattern) {
                Ok(glob) => glob_builder.add(glob),
                Err(error) => {
                    self.error_msg =
                        Some(format!("Ungültiges Ausschlussmuster '{pattern}': {error}"));
                    return;
                }
            };
        }
        let ignore = match glob_builder.build() {
            Ok(globs) => globs,
            Err(error) => {
                self.error_msg = Some(format!(
                    "Ausschlussmuster konnten nicht erstellt werden: {error}"
                ));
                return;
            }
        };
        let a = crate::vfs::sync_backend(a);
        let b = crate::vfs::sync_backend(b);
        let pair = crate::bisync::pair_id_for(&*a, &root_a, &*b, &root_b);
        let context = BisyncCtx {
            a: a.clone(),
            root_a: root_a.clone(),
            b: b.clone(),
            root_b: root_b.clone(),
            pair,
            baseline: crate::bisync::Baseline::new(),
        };
        let (tx, rx) = unbounded();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_t = cancel.clone();
        let spawn = std::thread::Builder::new()
            .name("bisync".into())
            .spawn(move || {
                let f = crate::bisync::WalkFilter {
                    include_hidden,
                    ignore: &ignore,
                    min_size: bounds.0,
                    max_size: bounds.1,
                    after_mtime_ms: bounds.2,
                    before_mtime_ms: bounds.3,
                };
                let _ = tx.send(crate::bisync::run(
                    &*a, &root_a, &*b, &root_b, opts, &cancel_t, &f,
                ));
            });
        match spawn {
            Ok(_) => {
                self.bisync_ctx = Some(context);
                self.bisync_cancel = Some(cancel);
                self.bisync_rx = Some(rx);
                self.bisync_running = true;
                self.running_job = job_id;
                self.notice = Some((
                    "⇄ 2-Wege-Sync läuft…".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.bisync_ctx = None;
                self.bisync_cancel = None;
                self.bisync_rx = None;
                self.bisync_running = false;
                self.running_job = None;
                self.error_msg = Some(format!("2-Wege-Sync-Thread konnte nicht starten: {error}"));
            }
        }
    }

    /// Backend + root for a tab index, honouring whether it's the focused tab
    /// (state in the App fields) or a parked split pane (state in `self.tabs`),
    /// and local vs. remote. Used by the split-view "sync these folders" action.
    pub(in crate::app) fn pane_backend(
        &self,
        tab_idx: usize,
    ) -> (crate::vfs::BackendHandle, String) {
        let (root, remote, net) = if tab_idx == self.active_tab {
            (&self.root_path, self.remote.as_ref(), self.net_conn.as_ref())
        } else {
            let tab = &self.tabs[tab_idx];
            (&tab.root_path, tab.remote.as_ref(), tab.net_conn.as_ref())
        };
        let backend: crate::vfs::BackendHandle = if let Some(remote) = remote {
            remote.backend.clone()
        } else if let Some(net) = net {
            Arc::new(crate::net::UncBackend::new(root, net.clone()))
        } else {
            Arc::new(crate::vfs::LocalBackend::new(root))
        };
        (crate::vfs::sync_backend(backend), root.clone())
    }

    /// Two-way sync the two split panes' folders (right-click action). Safe
    /// defaults; works across local/remote since each pane's live backend is
    /// reused directly.
    pub(in crate::app) fn sync_split_panes(&mut self) {
        if !self.split {
            return;
        }
        let (a_idx, b_idx) = (self.panes[0], self.panes[1]);
        let (a, root_a) = self.pane_backend(a_idx);
        let (b, root_b) = self.pane_backend(b_idx);
        if root_a.is_empty() || root_b.is_empty() {
            self.error_msg = Some("Beide Fenster müssen einen Ordner geöffnet haben.".to_string());
            return;
        }
        self.launch_bisync(
            a,
            root_a,
            b,
            root_b,
            crate::bisync::BisyncOptions::default(),
            true,
            Vec::new(),
            (0, 0, 0, 0),
            None,
        );
    }
}
