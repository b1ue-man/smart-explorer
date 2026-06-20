use super::prelude::*;
use super::*;

impl App {
    /// Deploy + activate the SSH remote agent on the ALREADY-connected SFTP
    /// session (runtime opt-in, #24) — no reconnect. Blocking deploy runs
    /// off-thread; the result is installed by `drain_agent_activate`.
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
        self.agent_activate_rx = Some(rx);
        self.agent_activate_for = Some(sftp.clone());
        self.notice = Some((
            "⚡ Aktiviere Remote-Agent…".to_string(),
            std::time::Instant::now(),
        ));
        std::thread::Builder::new()
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
            })
            .ok();
    }

    pub(in crate::app) fn drain_agent_activate(&mut self) {
        let res = match self
            .agent_activate_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            Some(r) => r,
            None => return,
        };
        self.agent_activate_rx = None;
        let target = self.agent_activate_for.take();
        match res {
            Ok((agent, ver)) => {
                // Install only if we're still on the same SFTP session.
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
                    // Persist so this connection auto-uses the agent next time.
                    if let Some(acc) = account {
                        let mut conns = crate::creds::load_connections();
                        if let Some(c) = conns.iter_mut().find(|c| c.account() == acc) {
                            c.use_agent = true;
                            let _ = crate::creds::save_connection(c);
                            self.saved_connections = crate::creds::load_connections();
                        }
                    }
                    self.notice = Some((
                        "⚡ Remote-Agent aktiv".to_string(),
                        std::time::Instant::now(),
                    ));
                    self.rescan();
                }
            }
            Err(e) => self.error_msg = Some(format!("Agent-Aktivierung: {e}")),
        }
    }

    /// Remove the remote agent from THIS connection: switch back to plain SFTP
    /// immediately (dropping the `AgentBackend` tears its bridge down → the
    /// remote `se-agent` process exits), un-persist the preference, and delete
    /// `~/.cache/smart-explorer` on the server (best-effort, off the UI thread).
    pub(in crate::app) fn remove_agent_now(&mut self) {
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
        if let Some(acc) = account {
            let mut conns = crate::creds::load_connections();
            if let Some(c) = conns.iter_mut().find(|c| c.account() == acc) {
                c.use_agent = false;
                let _ = crate::creds::save_connection(c);
                self.saved_connections = crate::creds::load_connections();
            }
        }
        std::thread::Builder::new()
            .name("agent-remove".into())
            .spawn(move || {
                let _ = crate::agent::remove_from_sftp(&sftp);
            })
            .ok();
        self.notice = Some((
            "Remote-Agent entfernt — Verbindung läuft wieder über SFTP".to_string(),
            std::time::Instant::now(),
        ));
    }

    /// One-time (cached) fetch of previously-released versions from the GitHub
    /// feed for the rollback list. No-op if already fetched / fetching.
    pub(in crate::app) fn fetch_remote_versions(&mut self) {
        if self.remote_versions.is_some() || self.remote_versions_rx.is_some() {
            return;
        }
        let (tx, rx) = unbounded();
        self.remote_versions_rx = Some(rx);
        std::thread::Builder::new()
            .name("versions-list".into())
            .spawn(move || {
                let _ = tx.send(crate::updater::list_remote_versions());
            })
            .ok();
    }

    /// Download a released version's binary (off-thread); installed by
    /// `drain_version_channels`. `forward` = a newer release to install as an
    /// update (no rollback pin); else a rollback to an older version.
    pub(in crate::app) fn start_version_download(&mut self, version: String, forward: bool) {
        if self.rollback_rx.is_some() {
            return;
        }
        let (tx, rx) = unbounded();
        self.rollback_rx = Some(rx);
        self.rollback_forward = forward;
        let verb = if forward { "Update" } else { "Lade" };
        self.notice = Some((format!("⬇ {verb} v{version} …"), std::time::Instant::now()));
        std::thread::Builder::new()
            .name("version-dl".into())
            .spawn(move || {
                let r = crate::updater::download_version(&version).map(|p| (version, p));
                let _ = tx.send(r);
            })
            .ok();
    }

    /// Download + roll back to an older released version.
    pub(in crate::app) fn start_rollback_download(&mut self, version: String) {
        self.start_version_download(version, false);
    }

    /// Download + install a newer released version as a forward update.
    pub(in crate::app) fn start_install_download(&mut self, version: String) {
        self.start_version_download(version, true);
    }

    pub(in crate::app) fn drain_version_channels(&mut self) {
        if let Some(rx) = self.remote_versions_rx.as_ref() {
            if let Ok(list) = rx.try_recv() {
                // The list is newest-first with the current version excluded, so
                // the first entry strictly newer than us is the update on offer.
                let current = env!("CARGO_PKG_VERSION");
                self.update_release_available = list
                    .iter()
                    .find(|v| crate::updater::is_newer(v, current))
                    .cloned();
                // Tell the user once, so a newer release is surfaced without
                // opening the update menu or pressing "Jetzt prüfen".
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
        }
        if let Some(rx) = self.rollback_rx.as_ref() {
            if let Ok(res) = rx.try_recv() {
                self.rollback_rx = None;
                let forward = self.rollback_forward;
                match res {
                    Ok((ver, exe)) => {
                        let applied = if forward {
                            crate::updater::install_version(&exe, &ver)
                        } else {
                            crate::updater::revert_to(&exe, &ver)
                        };
                        match applied {
                            Ok(cur) => {
                                if forward {
                                    self.update_release_available = None;
                                }
                                self.update_ready = Some((ver, cur));
                            }
                            Err(e) => {
                                let what = if forward { "Update" } else { "Zurückrollen" };
                                self.error_msg = Some(format!("{what}: {e}"));
                            }
                        }
                    }
                    Err(e) => self.error_msg = Some(format!("Version laden: {e}")),
                }
            }
        }
    }

    pub(in crate::app) fn drain_remote_op(&mut self) {
        let res = match self.remote_op_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(r) => r,
            None => return,
        };
        self.remote_op_rx = None;
        match res {
            Ok(msg) => {
                self.notice = Some((msg, std::time::Instant::now()));
                self.rescan();
            }
            // The worker already includes the operation context in the message.
            Err(e) => self.error_msg = Some(e),
        }
    }

    /// Our own right-click menu for a remote entry (the Windows shell menu can't
    /// act on remote paths). Each action routes through the backend.
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
                // Browse for the local destination in the in-app picker; the
                // download starts when the user confirms a folder.
                let _ = name;
                self.open_picker(PickerPurpose::DownloadTo { src: path }, "");
            }
        }
    }

    pub(in crate::app) fn drain_upload(&mut self) {
        let rx = match self.upload_rx.as_ref() {
            Some(rx) => rx,
            None => return,
        };
        let mut done: Option<(TransferProgress, Vec<String>)> = None;
        for _ in 0..16 {
            match rx.try_recv() {
                Ok(TransferMsg::Progress(progress)) => {
                    self.transfer_progress = Some(progress);
                }
                Ok(TransferMsg::Done { progress, errors }) => {
                    self.transfer_progress = Some(progress.clone());
                    done = Some((progress, errors));
                    break;
                }
                Err(_) => break,
            }
        }
        let (progress, errors) = match done {
            Some(done) => done,
            None => return,
        };
        self.upload_rx = None;
        self.transfer_progress = None;
        if !errors.is_empty() {
            self.error_msg = Some(format!(
                "Übertragung: {} Fehler (z. B. {})",
                errors.len(),
                errors[0]
            ));
        }
        self.notice = Some((
            format!("✓ {} übertragen", progress.files_done),
            std::time::Instant::now(),
        ));
        // Show the newly uploaded files.
        if self.remote.is_some() && !self.root_path.is_empty() {
            self.rescan();
        }
    }
}
