use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn maybe_save_index(&mut self) {
        if !self.index_dirty || self.index_last_saved.elapsed().as_secs() < 30 {
            return;
        }
        let mut buf = String::with_capacity(self.folder_index.len() * 50);
        for p in self.folder_index.iter() {
            buf.push_str(p);
            buf.push('\n');
        }
        let target = folder_index_path();
        std::thread::Builder::new()
            .name("index-save".into())
            .spawn(move || {
                let tmp = target.with_extension("txt.tmp");
                if std::fs::write(&tmp, buf).is_ok() {
                    let _ = std::fs::rename(&tmp, &target);
                }
            })
            .ok();
        self.index_dirty = false;
        self.index_last_saved = std::time::Instant::now();
    }

    // ─── Channel drains ─────────────────────────────────────────────────

    pub(in crate::app) fn drain_scan(&mut self) {
        let rx = match self.scan_rx.take() {
            Some(r) => r,
            None => return,
        };
        let (got_entries, got_done) = drain_scan_channel(
            &rx,
            &mut self.entries,
            &mut self.progress,
            &mut self.failed_paths,
            &mut self.error_msg,
        );
        if got_done {
            self.scan_handle = None;
            self.scan_running = false;
            self.recompute_view();
        } else {
            self.scan_rx = Some(rx);
            if got_entries {
                self.view_dirty = true;
            }
        }
    }

    /// Keep background tabs' scans flowing so their channels don't pile up
    /// unboundedly; their views are rebuilt lazily on activation.
    pub(in crate::app) fn drain_inactive_tabs(&mut self) {
        let active = self.active_tab;
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if i == active {
                continue;
            }
            if let Some(rx) = t.scan_rx.take() {
                let mut err = None;
                let (got_entries, got_done) = drain_scan_channel(
                    &rx,
                    &mut t.entries,
                    &mut t.progress,
                    &mut t.failed_paths,
                    &mut err,
                );
                if got_done {
                    t.scan_handle = None;
                    t.scan_running = false;
                    t.view_dirty = true;
                } else {
                    t.scan_rx = Some(rx);
                    if got_entries {
                        t.view_dirty = true;
                    }
                }
            }
        }
    }

    pub(in crate::app) fn drain_copy(&mut self) {
        let rx = match self.copy_rx.as_ref() {
            Some(r) => r,
            None => return,
        };
        let mut done = false;
        for _ in 0..16 {
            match rx.try_recv() {
                Ok(CopyMsg::Progress(p)) => self.copy_progress = Some(p),
                Ok(CopyMsg::Done { progress, errors }) => {
                    self.copy_progress = Some(progress);
                    self.copy_errors = errors;
                    done = true;
                    break;
                }
                Err(_) => break,
            }
        }
        if done {
            self.copy_rx = None;
            self.copy_handle = None;
            if !self.copy_errors.is_empty() {
                self.error_msg = Some(format!(
                    "{} Fehler beim Kopieren — erste: {}",
                    self.copy_errors.len(),
                    self.copy_errors
                        .first()
                        .map(|(p, m)| format!("{} ({})", p, m))
                        .unwrap_or_default()
                ));
            }
            if self.copy_refresh_after {
                self.copy_refresh_after = false;
                self.rescan();
            }
        }
    }

    pub(in crate::app) fn drain_trash(&mut self) {
        let mut msg: Option<Option<String>> = None;
        if let Some(rx) = self.trash_rx.as_ref() {
            if let Ok(m) = rx.try_recv() {
                msg = Some(m);
            }
        }
        if let Some(m) = msg {
            self.trash_rx = None;
            match m {
                None => {
                    self.notice = Some((
                        "✓ In Papierkorb verschoben".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Some(e) => {
                    self.error_msg = Some(format!("Papierkorb: {}", e));
                    // State may be out of sync with disk — refresh.
                    self.rescan();
                }
            }
        }
    }

    #[cfg(windows)]
    pub(in crate::app) fn drain_clip_prepare(&mut self) {
        let mut files = None;
        if let Some(rx) = self.clip_prepare_rx.as_ref() {
            if let Ok(f) = rx.try_recv() {
                files = Some(f);
            }
        }
        let files = match files {
            Some(f) => f,
            None => return,
        };
        self.clip_prepare_rx = None;
        if files.is_empty() {
            self.notice = Some((
                "Keine Dateien entsprechen dem aktiven Filter".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let pairs: Vec<(String, String)> = files
            .iter()
            .map(|f| (f.abs.clone(), f.rel.clone()))
            .collect();
        let n = files.len();
        match crate::virtual_clipboard::set_clipboard(files) {
            Ok(seq) => {
                self.virtual_clip = Some((seq, pairs));
                self.notice = Some((
                    format!(
                        "✓ {} gefilterte Datei(en) kopiert — Einfügen (auch im Explorer) erhält die Ordnerstruktur",
                        n
                    ),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.error_msg = Some(format!("Zwischenablage: {}", e));
            }
        }
    }

    #[cfg(not(windows))]
    pub(in crate::app) fn drain_clip_prepare(&mut self) {}

    pub(in crate::app) fn drain_update(&mut self) {
        use crate::updater::UpdateMsg;
        let mut msg = None;
        if let Some(rx) = self.update_rx.as_ref() {
            if let Ok(m) = rx.try_recv() {
                msg = Some(m);
            }
        }
        let msg = match msg {
            Some(m) => m,
            None => return,
        };
        self.update_rx = None;
        match msg {
            UpdateMsg::AppliedViaWorker { version } => {
                // The exe couldn't be replaced in place; a worker will do it
                // after we exit. Sentinel empty path = "just close, don't
                // relaunch" (the worker relaunches).
                self.notice = Some((
                    format!("⬆ Update auf v{} bereit (Neustart wendet es an)", version),
                    std::time::Instant::now(),
                ));
                self.update_ready = Some((version, PathBuf::new()));
            }
            UpdateMsg::UpToDate { feed_version } => {
                self.notice = Some((
                    format!(
                        "✓ Aktuell: v{} (Feed: v{})",
                        env!("CARGO_PKG_VERSION"),
                        feed_version
                    ),
                    std::time::Instant::now(),
                ));
            }
            UpdateMsg::NoFeed => {
                self.notice = Some((
                    "Kein Update-Feed konfiguriert (Pfad unten eintragen)".to_string(),
                    std::time::Instant::now(),
                ));
            }
            UpdateMsg::Error(e) => {
                self.error_msg = Some(format!("Update: {}", e));
            }
        }
    }

    pub(in crate::app) fn check_updates_manual(&mut self) {
        let (tx, rx) = unbounded();
        self.update_rx = Some(rx);
        crate::updater::check_async(tx, true);
    }

    // ─── Remote connections ─────────────────────────────────────────────

    /// Start connecting with the current form (off the UI thread).
    pub(in crate::app) fn begin_connect(
        &mut self,
        form: crate::connect::ConnectForm,
        secret: Option<String>,
    ) {
        self.connecting = true;
        self.error_msg = None;
        self.connect_rx = Some(crate::connect::spawn_connect(form, secret));
    }

    /// Connect to a saved connection: pre-fill from metadata + load its secret.
    pub(in crate::app) fn connect_saved(&mut self, c: &crate::creds::SavedConnection) {
        let form = crate::connect::ConnectForm::from_saved(c);
        let secret = crate::creds::get_secret(&c.account());
        // Bump to most-recent so the sidebar keeps the freshest connections up
        // front and overflows the stale ones into the menu.
        crate::creds::touch_connection(&c.account());
        self.saved_connections = crate::creds::load_connections();
        self.begin_connect(form, secret);
    }

    pub(in crate::app) fn drain_connect(&mut self) {
        let msg = match self.connect_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(m) => m,
            None => return,
        };
        self.connect_rx = None;
        self.connecting = false;
        match msg {
            crate::connect::ConnectResult::Ok(c) => {
                // SFTP/FTP set a remote backend; a share clears it (browsed
                // locally) but keeps the auth connection alive. Wrap remote
                // backends with the browsing cache (see `cache_remote`).
                self.remote = c.remote.map(|mut rs| {
                    rs.backend = cache_remote(rs.backend);
                    rs
                });
                if let Some(nc) = c.net {
                    self.net_conn = Some(nc);
                }
                self.show_connect = false;
                // A "save" during connect wrote connections.txt on the worker
                // thread; refresh the cached list so it shows immediately.
                self.saved_connections = crate::creds::load_connections();
                self.notice = Some((
                    format!("✓ Verbunden: {}", c.label),
                    std::time::Instant::now(),
                ));
                let pb = PathBuf::from(c.target.replace('/', std::path::MAIN_SEPARATOR_STR));
                self.start_scan(pb);
            }
            crate::connect::ConnectResult::Err(e) => {
                self.error_msg = Some(format!("Verbindung fehlgeschlagen: {}", e));
            }
        }
    }
}
