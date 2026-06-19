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
        let msg = match self.sync_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(m) => m,
            None => return,
        };
        match msg {
            crate::sync::SyncMsg::Progress(_) => {}
            crate::sync::SyncMsg::Done(r) => {
                self.sync_rx = None;
                self.sync_running = false;
                self.sync_cancel = None;
                if r.stats.errors > 0 {
                    self.error_msg = Some(format!(
                        "Spiegelung: {} kopiert, {} Fehler",
                        r.stats.copied, r.stats.errors
                    ));
                }
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

    /// Two-way sync the current location with `dest_local` (safe defaults: both
    /// directions, strict file-level conflicts, reversible, 30-day version
    /// retention). Conflicts come back for resolution.
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

    /// The single two-way-sync launcher used by the ad-hoc button, saved jobs,
    /// and the split-view "sync these two folders" action. Builds the ignore
    /// globset inside the worker (GlobSet isn't `Send`-cheap to pass), runs
    /// `bisync::run`, and stamps `running_job` so completion can mark the job.
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
        if self.bisync_running {
            self.notice = Some((
                "Es läuft bereits ein Sync — bitte warten.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let pair = crate::bisync::pair_id(&root_a, &root_b);
        self.bisync_ctx = Some(BisyncCtx {
            a: a.clone(),
            root_a: root_a.clone(),
            b: b.clone(),
            root_b: root_b.clone(),
            pair,
            baseline: crate::bisync::Baseline::new(),
        });
        let (tx, rx) = unbounded();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_t = cancel.clone();
        std::thread::Builder::new()
            .name("bisync".into())
            .spawn(move || {
                let mut gb = globset::GlobSetBuilder::new();
                for pat in &ignore {
                    let pat = pat.trim();
                    if pat.is_empty() {
                        continue;
                    }
                    if let Ok(g) = globset::Glob::new(pat) {
                        gb.add(g);
                    }
                }
                let gs = gb
                    .build()
                    .unwrap_or_else(|_| crate::bisync::empty_globset());
                let f = crate::bisync::WalkFilter {
                    include_hidden,
                    ignore: &gs,
                    min_size: bounds.0,
                    max_size: bounds.1,
                    after_mtime_ms: bounds.2,
                    before_mtime_ms: bounds.3,
                };
                let _ = tx.send(crate::bisync::run(
                    &*a, &root_a, &*b, &root_b, opts, &cancel_t, &f,
                ));
            })
            .ok();
        self.bisync_cancel = Some(cancel);
        self.bisync_rx = Some(rx);
        self.bisync_running = true;
        self.running_job = job_id;
        self.notice = Some((
            "⇄ 2-Wege-Sync läuft…".to_string(),
            std::time::Instant::now(),
        ));
    }

    /// Run a saved sync setup now. Local↔local resolves instantly; if either
    /// endpoint is a saved-connection remote URL it's re-opened off the UI
    /// thread first (so the window doesn't freeze), then launched.
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
        let opts = job.opts(false);
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
                job.filter_bounds(now_secs_i64()),
                Some(job.id.clone()),
            );
            return;
        }
        // Remote endpoint(s): re-open the saved connection(s) off-thread.
        let (src, tgt) = (job.source.clone(), job.target.clone());
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("job-connect".into())
            .spawn(move || {
                let res = (|| {
                    let a = crate::connect::resolve_endpoint(&src)?;
                    let b = crate::connect::resolve_endpoint(&tgt)?;
                    Ok::<_, String>((a, b))
                })();
                let _ = tx.send(res);
            })
            .ok();
        self.job_connect_rx = Some(rx);
        self.job_connect_pending = Some(job);
        self.notice = Some((
            "Verbinde mit Remote-Ziel…".to_string(),
            std::time::Instant::now(),
        ));
    }

    /// Once a remote job's endpoints are open, launch the sync (UI thread).
    pub(in crate::app) fn drain_job_connect(&mut self) {
        let res = match self
            .job_connect_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            Some(r) => r,
            None => return,
        };
        self.job_connect_rx = None;
        let job = match self.job_connect_pending.take() {
            Some(j) => j,
            None => return,
        };
        match res {
            Ok(((a, root_a), (b, root_b))) => {
                let opts = job.opts(false);
                self.launch_bisync(
                    a,
                    root_a,
                    b,
                    root_b,
                    opts,
                    job.include_hidden,
                    job.ignore.clone(),
                    job.filter_bounds(now_secs_i64()),
                    Some(job.id.clone()),
                );
            }
            Err(e) => {
                self.error_msg = Some(format!("Remote-Sync: {}", e));
            }
        }
    }

    /// Result of an interactive cloud authorize (#19, slice 1).
    pub(in crate::app) fn drain_cloud_auth(&mut self) {
        let res = match self
            .cloud_auth_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            Some(r) => r,
            None => return,
        };
        self.cloud_auth_rx = None;
        self.cloud_authing = false;
        match res {
            Ok(()) => {
                self.notice = Some((
                    "✓ Google Drive verbunden".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.error_msg = Some(format!("Cloud-Anmeldung: {}", e));
            }
        }
    }

    /// Open Google Drive as the active remote and browse it (reuses the normal
    /// connect drain → sidebar/scan path). Connects off the UI thread.
    pub(in crate::app) fn open_gdrive_browse(&mut self) {
        if !crate::cloud::is_connected(crate::cloud::Provider::GDrive) {
            self.error_msg = Some("Google Drive ist nicht verbunden.".to_string());
            return;
        }
        let (tx, rx) = unbounded();
        self.connect_rx = Some(rx);
        self.connecting = true;
        std::thread::Builder::new()
            .name("gdrive-open".into())
            .spawn(move || {
                let res = match crate::connect::open_gdrive("/") {
                    Ok((be, root)) => {
                        crate::connect::ConnectResult::Ok(crate::connect::Connected {
                            remote: Some(crate::connect::RemoteState {
                                backend: be,
                                label: "Google Drive".to_string(),
                                agent_version: None,
                                zip_return: None,
                                sftp: None,
                                account: None,
                                endpoint_prefix: Some("gdrive://".to_string()),
                            }),
                            net: None,
                            target: root,
                            label: "Google Drive".to_string(),
                        })
                    }
                    Err(e) => crate::connect::ConnectResult::Err(e),
                };
                let _ = tx.send(res);
            })
            .ok();
        self.notice = Some((
            "Verbinde mit Google Drive…".to_string(),
            std::time::Instant::now(),
        ));
    }

    /// Open Google Drive inside the picker (so a sync folder can be chosen on
    /// Drive). Connects off the UI thread via the picker's connect channel.
    pub(in crate::app) fn picker_open_gdrive(&mut self) {
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("gdrive-pick".into())
            .spawn(move || {
                let res = match crate::connect::open_gdrive("/") {
                    Ok((be, root)) => {
                        crate::connect::ConnectResult::Ok(crate::connect::Connected {
                            remote: Some(crate::connect::RemoteState {
                                backend: be,
                                label: "Google Drive".to_string(),
                                agent_version: None,
                                zip_return: None,
                                sftp: None,
                                account: None,
                                endpoint_prefix: Some("gdrive://".to_string()),
                            }),
                            net: None,
                            target: root,
                            label: "Google Drive".to_string(),
                        })
                    }
                    Err(e) => crate::connect::ConnectResult::Err(e),
                };
                let _ = tx.send(res);
            })
            .ok();
        if let Some(p) = self.picker.as_mut() {
            p.connect_rx = Some(rx);
            p.connecting = true;
            p.is_remote = true;
            p.endpoint_prefix = "gdrive://".to_string();
            p.conn_label = "Google Drive".to_string();
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
