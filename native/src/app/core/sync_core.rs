use super::prelude::*;
use super::*;

impl App {
    /// One-way mirror the current location (local or remote) into `dest_local`.
    pub(in crate::app) fn start_mirror(&mut self, dest_local: String) {
        if self.root_path.is_empty() || self.sync_running {
            return;
        }
        let src: crate::vfs::BackendHandle = match &self.remote {
            Some(rs) => rs.backend.clone(),
            None => Arc::new(crate::vfs::LocalBackend::new(&self.root_path)),
        };
        let dst: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new(&dest_local));
        let (tx, rx) = unbounded();
        let h = crate::sync::start_sync(
            src,
            self.root_path.clone(),
            dst,
            dest_local,
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
                if r.stats.errors > 0 {
                    let example = r
                        .errors
                        .first()
                        .map(|(path, detail)| format!(" ({path}: {detail})"))
                        .unwrap_or_default();
                    self.error_msg = Some(format!(
                        "Spiegelung unvollständig: {} kopiert, {} Fehler{}",
                        r.stats.copied, r.stats.errors, example
                    ));
                } else if canceled {
                    self.notice = Some((
                        format!("Spiegelung abgebrochen: {} bereits kopiert", r.stats.copied),
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
    pub(in crate::app) fn start_bisync(&mut self, dest_local: String) {
        if self.root_path.is_empty() {
            return;
        }
        let a: crate::vfs::BackendHandle = match &self.remote {
            Some(rs) => rs.backend.clone(),
            None => Arc::new(crate::vfs::LocalBackend::new(&self.root_path)),
        };
        let root_a = self.root_path.clone();
        let b: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new(&dest_local));
        self.launch_bisync(
            a,
            root_a,
            b,
            dest_local,
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

    /// Resolve any remote endpoints off-thread, then run a saved sync setup.
    pub(in crate::app) fn run_job(&mut self, id: &str) {
        if self.bisync_running || self.job_connect_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits ein Sync — bitte warten.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let job = match self.sync_jobs.iter().find(|j| j.id == id) {
            Some(j) => j.clone(),
            None => return,
        };
        let (opts, bounds) = match checked_job_settings(&job) {
            Ok(settings) => settings,
            Err(error) => {
                self.error_msg = Some(format!("Ungültiges Sync-Setup: {error}"));
                return;
            }
        };
        // Pure local: resolve inline (no network) and launch immediately.
        if !crate::connect::is_remote_url(&job.source)
            && !crate::connect::is_remote_url(&job.target)
        {
            let a: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new(&job.source));
            let b: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new(&job.target));
            self.launch_bisync(
                a,
                job.source.clone(),
                b,
                job.target.clone(),
                opts,
                job.include_hidden,
                job.ignore.clone(),
                bounds,
                Some(job.id.clone()),
            );
            return;
        }
        // Remote endpoint(s): re-open the saved connection(s) off-thread.
        let (src, tgt) = (job.source.clone(), job.target.clone());
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("job-connect".into())
            .spawn(move || {
                let res = (|| {
                    let a = crate::connect::resolve_endpoint(&src)?;
                    let b = crate::connect::resolve_endpoint(&tgt)?;
                    Ok::<_, String>((a, b))
                })();
                let _ = tx.send(res);
            });
        match spawn {
            Ok(_) => {
                self.job_connect_rx = Some(rx);
                self.job_connect_pending = Some(job);
                self.notice = Some((
                    "Verbinde mit Remote-Ziel…".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.job_connect_rx = None;
                self.job_connect_pending = None;
                self.error_msg = Some(format!(
                    "Remote-Sync-Verbindung konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    /// Once a remote job's endpoints are open, launch the sync (UI thread).
    pub(in crate::app) fn drain_job_connect(&mut self) {
        let res = match self.job_connect_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.job_connect_rx = None;
                self.job_connect_pending = None;
                self.error_msg =
                    Some("Remote-Sync-Verbindung wurde ohne Ergebnis beendet.".to_string());
                return;
            }
        };
        self.job_connect_rx = None;
        let job = match self.job_connect_pending.take() {
            Some(j) => j,
            None => {
                self.error_msg = Some("Remote-Sync-Auftrag fehlt.".to_string());
                return;
            }
        };
        match res {
            Ok(((a, root_a), (b, root_b))) => {
                let (opts, bounds) = match checked_job_settings(&job) {
                    Ok(settings) => settings,
                    Err(error) => {
                        self.error_msg = Some(format!("Ungültiges Sync-Setup: {error}"));
                        return;
                    }
                };
                self.launch_bisync(
                    a,
                    root_a,
                    b,
                    root_b,
                    opts,
                    job.include_hidden,
                    job.ignore.clone(),
                    bounds,
                    Some(job.id.clone()),
                );
            }
            Err(e) => {
                self.error_msg = Some(format!("Remote-Sync: {}", e));
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
        if tab_idx == self.active_tab {
            let root = self.root_path.clone();
            let be: crate::vfs::BackendHandle = match &self.remote {
                Some(rs) => rs.backend.clone(),
                None => Arc::new(crate::vfs::LocalBackend::new(&root)),
            };
            (be, root)
        } else {
            let t = &self.tabs[tab_idx];
            let root = t.root_path.clone();
            let be: crate::vfs::BackendHandle = match &t.remote {
                Some(rs) => rs.backend.clone(),
                None => Arc::new(crate::vfs::LocalBackend::new(&root)),
            };
            (be, root)
        }
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
        if root_a == root_b {
            self.error_msg = Some("Beide Fenster zeigen denselben Ordner.".to_string());
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

type SyncFilterBounds = (u64, u64, i64, i64);
type CheckedJobSettings = (crate::bisync::BisyncOptions, SyncFilterBounds);

fn checked_job_settings(job: &crate::syncjobs::SyncJob) -> Result<CheckedJobSettings, String> {
    job.validate()?;
    Ok((
        job.checked_opts(false)?,
        job.checked_filter_bounds(now_secs_i64())?,
    ))
}
