use super::prelude::*;
use super::*;

impl App {
    /// Open the picker to fill a sync-setup field, starting from `initial`
    /// (local path → browse there; remote URL or empty → start at the roots).
    pub(in crate::app) fn open_picker(&mut self, purpose: PickerPurpose, initial: &str) {
        let mut st = PickerState {
            purpose,
            backend: None,
            is_remote: false,
            endpoint_prefix: String::new(),
            conn_label: String::new(),
            cwd: String::new(),
            entries: Vec::new(),
            error: None,
            connect_rx: None,
            connecting: false,
        };
        // A local starting folder opens directly; remote/empty starts at roots.
        if !initial.trim().is_empty()
            && !crate::connect::is_remote_url(initial)
            && is_local_style(initial)
        {
            st.backend = Some(Arc::new(crate::vfs::LocalBackend::new("/")));
            st.cwd = initial.replace('\\', "/").trim_end_matches('/').to_string();
            if st.cwd.is_empty() {
                st.cwd = "/".into();
            }
        }
        self.picker = Some(st);
        if self
            .picker
            .as_ref()
            .map(|s| s.backend.is_some())
            .unwrap_or(false)
        {
            self.picker_list();
        }
    }

    /// (Re)list the current picker folder via its backend (folders only).
    pub(in crate::app) fn picker_list(&mut self) {
        let (backend, cwd) = match &self.picker {
            Some(p) => match &p.backend {
                Some(b) => (b.clone(), ensure_dir_root(&p.cwd)),
                None => return,
            },
            None => return,
        };
        let res = backend.list_dir(&cwd);
        if let Some(p) = self.picker.as_mut() {
            match res {
                Ok(metas) => {
                    let mut dirs: Vec<String> = metas
                        .into_iter()
                        .filter(|m| m.is_dir)
                        .map(|m| m.name)
                        .collect();
                    dirs.sort_by_key(|n| n.to_lowercase());
                    p.entries = dirs;
                    p.error = None;
                }
                Err(e) => {
                    p.entries.clear();
                    p.error = Some(e.to_string());
                }
            }
        }
    }

    /// Open a local drive / folder root in the picker.
    pub(in crate::app) fn picker_open_local(&mut self, root: &str) {
        if let Some(p) = self.picker.as_mut() {
            p.backend = Some(Arc::new(crate::vfs::LocalBackend::new("/")));
            p.is_remote = false;
            p.endpoint_prefix = String::new();
            p.conn_label = String::new();
            let c = root.replace('\\', "/");
            let c = c.trim_end_matches('/');
            p.cwd = if c.is_empty() {
                "/".into()
            } else {
                ensure_dir_root(c)
            };
            p.connecting = false;
            p.connect_rx = None;
        }
        self.picker_list();
    }

    /// Open a saved connection in the picker (async connect; keeps creds).
    pub(in crate::app) fn picker_open_connection(&mut self, c: &crate::creds::SavedConnection) {
        let form = crate::connect::ConnectForm::from_saved(c);
        let secret = crate::creds::get_secret(&c.account());
        let rx = crate::connect::spawn_connect(form, secret);
        if let Some(p) = self.picker.as_mut() {
            p.connect_rx = Some(rx);
            p.connecting = true;
            p.error = None;
            p.conn_label = c.display();
            p.is_remote = c.protocol.is_url();
            p.endpoint_prefix = if c.protocol.is_url() {
                format!("{}://{}@{}:{}", c.protocol.as_str(), c.user, c.host, c.port)
            } else {
                String::new()
            };
        }
    }

    pub(in crate::app) fn drain_picker_connect(&mut self) {
        let msg = match self
            .picker
            .as_ref()
            .and_then(|p| p.connect_rx.as_ref())
            .and_then(|rx| rx.try_recv().ok())
        {
            Some(m) => m,
            None => return,
        };
        let mut do_list = false;
        if let Some(p) = self.picker.as_mut() {
            p.connect_rx = None;
            p.connecting = false;
            match msg {
                crate::connect::ConnectResult::Ok(c) => {
                    // SFTP/FTP/WebDAV → remote backend; share → browse the UNC
                    // locally once authenticated.
                    if let Some(rs) = c.remote {
                        p.backend = Some(cache_remote(rs.backend));
                        p.is_remote = true;
                    } else {
                        p.backend = Some(Arc::new(crate::vfs::LocalBackend::new(&c.target)));
                        p.is_remote = false;
                        p.endpoint_prefix = String::new();
                    }
                    p.cwd = c.target;
                    do_list = true;
                }
                crate::connect::ConnectResult::Err(e) => {
                    p.error = Some(format!("Verbindung fehlgeschlagen: {}", e));
                }
            }
        }
        if do_list {
            self.picker_list();
        }
    }

    /// Parent of a picker directory (None at a drive/remote root).
    pub(in crate::app) fn picker_parent(p: &str) -> Option<String> {
        let t = p.trim_end_matches('/');
        if t.is_empty() || t == "/" {
            return None;
        }
        if t.len() == 2 && t.ends_with(':') {
            return None; // drive root "C:"
        }
        match t.rsplit_once('/') {
            Some((par, _)) => {
                if par.is_empty() {
                    Some("/".into())
                } else if par.len() == 2 && par.ends_with(':') {
                    Some(format!("{}/", par))
                } else {
                    Some(par.to_string())
                }
            }
            None => None,
        }
    }

    /// The value the picker would return for the current folder.
    pub(in crate::app) fn picker_value(p: &PickerState) -> String {
        if p.is_remote {
            format!("{}{}", p.endpoint_prefix, p.cwd)
        } else {
            p.cwd.clone()
        }
    }

    pub(in crate::app) fn ui_picker(&mut self, ctx: &egui::Context) {
        if self.picker.is_none() {
            return;
        }
        let mut open = true;
        let mut close = false;
        let mut choose = false;
        let mut enter: Option<String> = None;
        let mut go_up = false;
        let mut open_local: Option<String> = None;
        let mut open_conn: Option<crate::creds::SavedConnection> = None;

        let Some(st) = self.picker.as_ref() else {
            return;
        };
        let title = st.purpose.title();
        let local_only = st.purpose.local_only();
        let home = self.home.to_string_lossy().replace('\\', "/");
        let drives = self.drive_info.clone();
        let conns = self.saved_connections.clone();
        let connecting = st.connecting;
        let error = st.error.clone();
        let cwd = st.cwd.clone();
        let entries = st.entries.clone();
        let conn_label = st.conn_label.clone();
        let value_preview = Self::picker_value(st);
        let has_loc = st.backend.is_some();
        let gdrive_connected = crate::cloud::is_connected(crate::cloud::Provider::GDrive);
        let mut open_gdrive = false;

        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([760.0, 560.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // ── Left: places ──
                    ui.vertical(|ui| {
                        ui.set_min_width(200.0);
                        ui.label(RichText::new("ORTE").small().color(Color32::from_gray(140)));
                        if ui.selectable_label(false, "🏠 Home").clicked() {
                            open_local = Some(home.clone());
                        }
                        for (d, _f, _t) in &drives {
                            if ui.selectable_label(false, format!("💽 {}", d)).clicked() {
                                open_local = Some(d.clone());
                            }
                        }
                        // Remote connections only for sync source/target.
                        if !local_only {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("VERBINDUNGEN")
                                    .small()
                                    .color(Color32::from_gray(140)),
                            );
                            if conns.is_empty() && !gdrive_connected {
                                ui.colored_label(Color32::from_gray(120), "(keine)");
                            }
                            if gdrive_connected
                                && ui.selectable_label(false, "☁ Google Drive").clicked()
                            {
                                open_gdrive = true;
                            }
                            for c in &conns {
                                if ui
                                    .selectable_label(false, format!("🖧 {}", c.display()))
                                    .on_hover_text(c.to_target())
                                    .clicked()
                                {
                                    open_conn = Some(c.clone());
                                }
                            }
                        }
                    });

                    ui.separator();

                    // ── Right: current folder ──
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            if ui.button("⬆ Hoch").clicked() {
                                go_up = true;
                            }
                            if !conn_label.is_empty() {
                                ui.colored_label(
                                    Color32::from_rgb(120, 200, 255),
                                    format!("● {}", conn_label),
                                );
                            }
                        });
                        ui.label(
                            RichText::new(if cwd.is_empty() {
                                "—".to_string()
                            } else {
                                cwd.clone()
                            })
                            .monospace()
                            .color(Color32::from_gray(180)),
                        );
                        ui.separator();
                        if connecting {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Verbinde…");
                            });
                        } else if let Some(e) = &error {
                            ui.colored_label(Color32::from_rgb(255, 140, 120), e);
                        } else if !has_loc {
                            ui.colored_label(
                                Color32::from_gray(140),
                                "Links einen Ort oder eine Verbindung wählen.",
                            );
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("picker_list")
                            .max_height(460.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for name in &entries {
                                    if ui
                                        .selectable_label(false, format!("📁 {}", name))
                                        .double_clicked()
                                    {
                                        enter = Some(name.clone());
                                    }
                                }
                                if has_loc && entries.is_empty() && error.is_none() && !connecting {
                                    ui.colored_label(
                                        Color32::from_gray(120),
                                        "(keine Unterordner)",
                                    );
                                }
                            });
                    });
                });

                ui.separator();
                ui.horizontal(|ui| {
                    let can_choose = has_loc && !connecting && !cwd.is_empty();
                    if ui
                        .add_enabled(can_choose, egui::Button::new("✔ Diesen Ordner wählen"))
                        .clicked()
                    {
                        choose = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        close = true;
                    }
                    if can_choose {
                        ui.colored_label(Color32::from_gray(140), value_preview.clone());
                    }
                });
            });

        // Apply deferred actions (outside the borrow of self.picker).
        if let Some(name) = enter {
            if let Some(p) = self.picker.as_mut() {
                p.cwd = format!("{}/{}", p.cwd.trim_end_matches('/'), name);
            }
            self.picker_list();
        }
        if go_up {
            let parent = self
                .picker
                .as_ref()
                .and_then(|p| Self::picker_parent(&p.cwd));
            if let Some(par) = parent {
                if let Some(p) = self.picker.as_mut() {
                    p.cwd = par;
                }
                self.picker_list();
            }
        }
        if let Some(root) = open_local {
            self.picker_open_local(&root);
        }
        if let Some(c) = open_conn {
            self.picker_open_connection(&c);
        }
        if open_gdrive {
            self.picker_open_gdrive();
        }
        if choose {
            if let Some(p) = self.picker.take() {
                let value = Self::picker_value(&p);
                let native = value.replace('/', std::path::MAIN_SEPARATOR_STR);
                match p.purpose {
                    PickerPurpose::SyncSource => {
                        if let Some(ed) = self.job_editor.as_mut() {
                            ed.source = value;
                        }
                    }
                    PickerPurpose::SyncTarget => {
                        if let Some(ed) = self.job_editor.as_mut() {
                            ed.target = value;
                        }
                    }
                    PickerPurpose::ScanFolder => self.start_scan(PathBuf::from(native)),
                    PickerPurpose::AnalyticsFolder => {
                        // Reuse the picker's live backend for a remote target;
                        // otherwise analyse the local folder.
                        if p.is_remote {
                            if let Some(be) = p.backend.clone() {
                                self.start_analytics_scan_remote(
                                    be,
                                    p.cwd.clone(),
                                    p.conn_label.clone(),
                                );
                            }
                        } else {
                            self.start_analytics_scan(value);
                        }
                    }
                    PickerPurpose::ReclaimFolder => self.start_reclaim_scan(value),
                    PickerPurpose::MirrorDest => self.start_mirror(value),
                    PickerPurpose::BisyncDest => self.start_bisync(value),
                    PickerPurpose::CopyDest => self.copy_dest = native,
                    PickerPurpose::DownloadTo { src } => {
                        if let Some(backend) = self.remote.as_ref().map(|rs| rs.backend.clone()) {
                            self.start_remote_download(backend, vec![src], native, None);
                        }
                    }
                }
            }
        } else if close || !open {
            self.picker = None;
        }
    }
}
