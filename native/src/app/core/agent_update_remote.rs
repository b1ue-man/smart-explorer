use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn start_agent_activation(&mut self) {
        if self.agent_activate_rx.is_some() {
            return;
        }
        let sftp = match self.remote.as_ref() {
            Some(rs) if rs.agent_version.is_none() => match &rs.sftp {
                Some(s) => s.clone(),
                None => return,
            },
            _ => return,
        };
        let (tx, rx) = unbounded();
        let target = sftp.clone();
        let spawn = std::thread::Builder::new()
            .name("agent-activate".into())
            .spawn(move || {
                let inner: crate::vfs::BackendHandle = sftp.clone();
                let r = crate::agent::deploy_over_sftp(&sftp, inner)
                    .map(|a| {
                        let v = a.version().to_string();
                        (a, v)
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(r);
            });
        match spawn {
            Ok(_) => {
                self.agent_activate_rx = Some(rx);
                self.agent_activate_for = Some(target);
                self.notice = Some((
                    "⚡ Aktiviere Remote-Agent…".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.agent_activate_rx = None;
                self.agent_activate_for = None;
                self.error_msg = Some(format!(
                    "Agent-Aktivierung konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    pub(in crate::app) fn drain_agent_activate(&mut self) {
        let res = match self.agent_activate_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.agent_activate_rx = None;
                self.agent_activate_for = None;
                self.error_msg = Some("Agent-Aktivierung wurde ohne Ergebnis beendet.".to_string());
                return;
            }
        };
        self.agent_activate_rx = None;
        let target = self.agent_activate_for.take();
        match res {
            Ok((agent, ver)) => {
                let same = matches!(
                    (self.remote.as_ref().and_then(|rs| rs.sftp.as_ref()), target.as_ref()),
                    (Some(a), Some(b)) if Arc::ptr_eq(a, b)
                );
                if same {
                    let account = self.remote.as_ref().and_then(|rs| rs.account.clone());
                    if let Some(rs) = self.remote.as_mut() {
                        rs.backend = cache_remote(Arc::new(agent));
                        rs.agent_version = Some(ver);
                    }
                    let persistence_error = persist_agent_preference(account.as_deref(), true);
                    self.saved_connections = crate::creds::load_connections();
                    if let Some(error) = persistence_error {
                        self.error_msg = Some(format!(
                            "Remote-Agent ist aktiv, aber die Einstellung wurde nicht gespeichert: {error}"
                        ));
                    } else {
                        self.notice = Some((
                            "⚡ Remote-Agent aktiv".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                    self.rescan();
                }
            }
            Err(e) => self.error_msg = Some(format!("Agent-Aktivierung: {e}")),
        }
    }

    /// Switch back to SFTP and remove the deployed agent off-thread.
    pub(in crate::app) fn remove_agent_now(&mut self) {
        if self.remote_op_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits ein Remote-Vorgang…".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let (sftp, account) = match self.remote.as_ref() {
            Some(rs) if rs.agent_version.is_some() => match &rs.sftp {
                Some(s) => (s.clone(), rs.account.clone()),
                None => return,
            },
            _ => return,
        };
        if let Some(rs) = self.remote.as_mut() {
            rs.backend = cache_remote(sftp.clone());
            rs.agent_version = None;
        }
        let persistence_error = persist_agent_preference(account.as_deref(), false);
        self.saved_connections = crate::creds::load_connections();
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("agent-remove".into())
            .spawn(move || {
                let result =
                    crate::agent::remove_from_sftp(&sftp)
                        .map_err(|error| format!("Agent entfernen: {error}"))
                        .and_then(|()| match persistence_error {
                            Some(error) => Err(format!(
                                "Remote-Agent entfernt, Einstellung nicht gespeichert: {error}"
                            )),
                            None => Ok("Remote-Agent entfernt — Verbindung läuft wieder über SFTP"
                                .to_string()),
                        });
                let _ = tx.send(result);
            });
        match spawn {
            Ok(_) => {
                self.remote_op_rx = Some(rx);
                self.notice = Some((
                    "Entferne Remote-Agent…".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.error_msg = Some(format!(
                    "Verbindung nutzt wieder SFTP, aber die Agent-Bereinigung konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    pub(in crate::app) fn fetch_remote_versions(&mut self) {
        if self.remote_versions.is_some() || self.remote_versions_rx.is_some() {
            return;
        }
        let (tx, rx) = unbounded();
        let spawn = std::thread::Builder::new()
            .name("versions-list".into())
            .spawn(move || {
                let _ = tx.send(crate::updater::list_remote_versions());
            });
        match spawn {
            Ok(_) => self.remote_versions_rx = Some(rx),
            Err(error) => {
                self.remote_versions = Some(Vec::new());
                self.error_msg = Some(format!(
                    "Versionsabfrage konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    pub(in crate::app) fn start_version_download(&mut self, version: String) {
        if self.rollback_rx.is_some() {
            return;
        }
        let (tx, rx) = unbounded();
        let notice = format!("⬇ Lade v{version} …");
        let spawn = std::thread::Builder::new()
            .name("version-dl".into())
            .spawn(move || {
                let r = crate::updater::download_version(&version).map(|p| (version, p));
                let _ = tx.send(r);
            });
        match spawn {
            Ok(_) => {
                self.rollback_rx = Some(rx);
                self.notice = Some((notice, std::time::Instant::now()));
            }
            Err(error) => {
                self.error_msg = Some(format!(
                    "Versionsdownload konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    pub(in crate::app) fn start_rollback_download(&mut self, version: String) {
        self.start_version_download(version);
    }

    pub(in crate::app) fn start_install_download(&mut self, version: String) {
        if self.update_rx.is_some() {
            self.notice = Some((
                "Eine Update-Prüfung läuft bereits…".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let (tx, rx) = unbounded();
        let shown_version = version.clone();
        let spawn = std::thread::Builder::new()
            .name("update-version-dl".into())
            .spawn(move || {
                let message = match crate::updater::download_update(&version) {
                    Ok(bundle) => crate::updater::UpdateMsg::Staged(bundle),
                    Err(error) => crate::updater::UpdateMsg::Error(error),
                };
                let _ = tx.send(message);
            });
        match spawn {
            Ok(_) => {
                self.update_rx = Some(rx);
                self.notice = Some((
                    format!("⬇ Stage Update v{shown_version} …"),
                    std::time::Instant::now(),
                ));
            }
            Err(error) => {
                self.error_msg = Some(format!(
                    "Update-Download konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    pub(in crate::app) fn drain_version_channels(&mut self) {
        match self.remote_versions_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(list)) => {
                let current = env!("CARGO_PKG_VERSION");
                self.update_release_available = list
                    .iter()
                    .find(|v| crate::updater::is_newer(v, current))
                    .cloned();
                if let Some(v) = self.update_release_available.clone() {
                    if !self.update_release_notified {
                        self.update_release_notified = true;
                        self.notice = Some((
                            format!("⬆ Update verfügbar: v{v} — im Update-Menü installierbar"),
                            std::time::Instant::now(),
                        ));
                    }
                }
                self.remote_versions = Some(list);
                self.remote_versions_rx = None;
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.remote_versions_rx = None;
                self.remote_versions = Some(Vec::new());
                self.error_msg = Some("Versionsabfrage wurde ohne Ergebnis beendet.".to_string());
            }
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => {}
        }
        match self.rollback_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(res)) => {
                self.rollback_rx = None;
                match res {
                    Ok((ver, exe)) => match crate::updater::revert_to(&exe, &ver) {
                        Ok(cur) => {
                            self.update_ready = Some(ReadyUpdate::InstalledRollback {
                                version: ver,
                                executable: cur,
                            });
                            self.show_update_dialog = true;
                        }
                        Err(e) => {
                            self.error_msg = Some(format!("Zurückrollen: {e}"));
                        }
                    },
                    Err(e) => self.error_msg = Some(format!("Version laden: {e}")),
                }
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.rollback_rx = None;
                self.error_msg = Some("Versionsdownload wurde ohne Ergebnis beendet.".to_string());
            }
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => {}
        }
    }

    pub(in crate::app) fn drain_remote_op(&mut self) {
        let res = match self.remote_op_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => return,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.remote_op_rx = None;
                self.error_msg = Some("Remote-Vorgang wurde ohne Ergebnis beendet.".to_string());
                self.rescan();
                return;
            }
        };
        self.remote_op_rx = None;
        match res {
            Ok(msg) => {
                self.notice = Some((msg, std::time::Instant::now()));
                self.rescan();
            }
            Err(e) => {
                self.error_msg = Some(e);
                // Some backends can report an error after the server already
                // committed the operation. Refresh so the visible state never
                // remains optimistically stale after an ambiguous failure.
                self.rescan();
            }
        }
    }

    pub(in crate::app) fn ui_remote_ctx(&mut self, ctx: &egui::Context) {
        let (pos, idx) = match self.remote_ctx {
            Some(v) => v,
            None => return,
        };
        if idx >= self.entries.len() {
            self.remote_ctx = None;
            return;
        }
        let e = &self.entries[idx];
        let path = e.path.to_string();
        let name = e.name.to_string();
        let is_dir = e.is_dir;
        let starred = is_dir && self.is_favorite(&self.location_key(&path));

        #[derive(Clone, Copy)]
        enum A {
            Open,
            OpenWith,
            DownloadTo,
            CopyClip,
            Rename,
            Delete,
            NewFolder,
            CopyPath,
            Refresh,
            Star,
        }
        let mut act: Option<A> = None;
        let area = egui::Area::new(egui::Id::new("remote_ctx_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    if ui
                        .button(if is_dir {
                            "📂 Öffnen"
                        } else {
                            "📄 Öffnen"
                        })
                        .clicked()
                    {
                        act = Some(A::Open);
                    }
                    if is_dir
                        && ui
                            .button(if starred {
                                "☆ Aus Favoriten entfernen"
                            } else {
                                "★ Zu Favoriten"
                            })
                            .clicked()
                    {
                        act = Some(A::Star);
                    }
                    if !is_dir {
                        if ui
                            .button("📂 Öffnen mit…")
                            .on_hover_text(
                                "Lädt die Datei lokal und öffnet Windows' „Öffnen mit“-Auswahl",
                            )
                            .clicked()
                        {
                            act = Some(A::OpenWith);
                        }
                        if ui.button("⬇ Herunterladen nach…").clicked() {
                            act = Some(A::DownloadTo);
                        }
                        if ui.button("📋 In Zwischenablage kopieren").clicked() {
                            act = Some(A::CopyClip);
                        }
                    }
                    if is_dir {
                        if ui.button("Herunterladen nach...").clicked() {
                            act = Some(A::DownloadTo);
                        }
                        if ui.button("In Zwischenablage kopieren").clicked() {
                            act = Some(A::CopyClip);
                        }
                    }
                    ui.separator();
                    if ui.button("✎ Umbenennen").clicked() {
                        act = Some(A::Rename);
                    }
                    if ui.button("🗑 Löschen").clicked() {
                        act = Some(A::Delete);
                    }
                    ui.separator();
                    if ui.button("＋ Neuer Ordner").clicked() {
                        act = Some(A::NewFolder);
                    }
                    if ui.button("⧉ Pfad kopieren").clicked() {
                        act = Some(A::CopyPath);
                    }
                    if ui.button("⟳ Aktualisieren").clicked() {
                        act = Some(A::Refresh);
                    }
                });
            });
        let dismiss = ctx.input(|i| i.key_pressed(egui::Key::Escape))
            || (ctx.input(|i| i.pointer.any_pressed())
                && ctx
                    .input(|i| i.pointer.interact_pos())
                    .map(|p| !area.response.rect.contains(p))
                    .unwrap_or(false));
        let act = match act {
            Some(a) => {
                self.remote_ctx = None;
                a
            }
            None => {
                if dismiss {
                    self.remote_ctx = None;
                }
                return;
            }
        };
        match act {
            A::Open => self.activate_entry(idx),
            A::OpenWith => self.open_with_entry(idx),
            A::Refresh => self.rescan(),
            A::NewFolder => self.create_new_folder(),
            A::Delete => self.trash_selected(),
            A::CopyClip => self.clipboard_copy_files(false),
            A::CopyPath => ctx.copy_text(path),
            A::Star => {
                let key = self.location_key(&path);
                self.toggle_favorite(&key);
            }
            A::Rename => {
                self.rename_open = Some((path, name));
                self.rename_focus = true;
            }
            A::DownloadTo => {
                let _ = name;
                self.open_picker(PickerPurpose::DownloadTo { src: path }, "");
            }
        }
    }
}

fn persist_agent_preference(account: Option<&str>, enabled: bool) -> Option<String> {
    let account = account?;
    let mut connections = crate::creds::load_connections();
    let Some(connection) = connections
        .iter_mut()
        .find(|connection| connection.account() == account)
    else {
        return Some("gespeicherte Verbindung nicht gefunden".to_string());
    };
    connection.use_agent = enabled;
    crate::creds::save_connection(connection)
        .err()
        .map(|error| error.to_string())
}
