use super::prelude::*;
use super::*;

impl App {
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
        let mut disconnected = false;
        for _ in 0..16 {
            match rx.try_recv() {
                Ok(CopyMsg::Progress(p)) => self.copy_progress = Some(p),
                Ok(CopyMsg::Done {
                    mut progress,
                    errors,
                }) => {
                    progress.done = true;
                    self.copy_progress = Some(progress);
                    self.copy_errors = errors;
                    done = true;
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected && !done {
            let message = "Kopier-Thread wurde ohne Ergebnis beendet.".to_string();
            let progress = self.copy_progress.get_or_insert(CopyProgress {
                files_done: 0,
                files_total: 0,
                bytes_done: 0,
                bytes_total: 0,
                elapsed_ms: 0,
                errors: 0,
                canceled: false,
                done: false,
            });
            progress.errors = progress.errors.saturating_add(1);
            progress.done = true;
            self.copy_errors
                .push(("Kopier-Worker".to_string(), message.clone()));
            self.error_msg = Some(message);
            let refresh = self.finish_copy_job();
            if refresh {
                self.rescan();
            }
            return;
        }
        if done {
            let canceled = matches!(&self.copy_progress, Some(p) if p.canceled);
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
            if canceled {
                self.notice = Some((
                    "Kopiervorgang abgebrochen; bereits abgeschlossene Dateien bleiben erhalten."
                        .to_string(),
                    std::time::Instant::now(),
                ));
            }
            let refresh = self.finish_copy_job();
            if refresh {
                self.rescan();
            }
        }
    }

    pub(in crate::app) fn drain_clip_prepare(&mut self) {
        let result = match self.clip_prepare_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.clip_prepare_rx = None;
                self.error_msg =
                    Some("Gefilterte Zwischenablage wurde ohne Ergebnis beendet.".to_string());
                return;
            }
        };
        self.clip_prepare_rx = None;
        let files = match result {
            Ok(files) => files,
            Err(error) => {
                self.error_msg = Some(error);
                return;
            }
        };
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
        match set_virtual_clipboard(files) {
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

    pub(in crate::app) fn drain_update(&mut self) {
        use crate::updater::UpdateMsg;
        let msg = match self.update_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(message)) => message,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.update_rx = None;
                self.error_msg = Some("Update-Prüfung wurde unerwartet beendet.".to_string());
                return;
            }
        };
        self.update_rx = None;
        match msg {
            UpdateMsg::Finished => {}
            UpdateMsg::Staged(bundle) => {
                let version = bundle.version().to_string();
                self.notice = Some((
                    format!("⬆ Update auf v{} bereit (Neustart wendet es an)", version),
                    std::time::Instant::now(),
                ));
                self.update_release_available = None;
                self.update_ready = Some(ReadyUpdate::Staged(bundle));
                self.show_update_dialog = true;
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
            UpdateMsg::BackgroundError(e) => {
                self.push_app_error("Automatische Update-Prüfung", e);
            }
        }
    }

    pub(in crate::app) fn check_updates_manual(&mut self) {
        let (tx, rx) = unbounded();
        match crate::updater::check_async(tx, true) {
            Ok(()) => self.update_rx = Some(rx),
            Err(error) => {
                self.update_rx = None;
                self.error_msg = Some(format!(
                    "Update-Prüfung konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    // ─── Remote connections ─────────────────────────────────────────────

    /// Start connecting with the current form (off the UI thread).
    pub(in crate::app) fn begin_connect(
        &mut self,
        form: crate::connect::ConnectForm,
        secret: Option<String>,
    ) {
        self.error_msg = None;
        match crate::connect::spawn_connect(form, secret) {
            Ok(rx) => {
                self.connect_rx = Some(rx);
                self.connecting = true;
            }
            Err(error) => {
                self.connect_rx = None;
                self.connecting = false;
                self.error_msg = Some(error);
            }
        }
    }

    /// Connect to a saved connection: pre-fill from metadata + load its secret.
    pub(in crate::app) fn connect_saved(&mut self, c: &crate::creds::SavedConnection) {
        let form = crate::connect::ConnectForm::from_saved(c);
        let secret = crate::creds::get_secret(&c.account());
        // Bump to most-recent so the sidebar keeps the freshest connections up
        // front and overflows the stale ones into the menu.
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

    pub(in crate::app) fn drain_connect(&mut self) {
        let msg = match self.connect_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(message)) => message,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.connect_rx = None;
                self.connecting = false;
                self.error_msg = Some("Verbindungs-Thread wurde ohne Ergebnis beendet.".into());
                return;
            }
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
