use super::prelude::*;
use super::*;

impl App {
    /// Start the share service if a rendezvous server is configured. Returns
    /// whether a service is available.
    pub(in crate::app) fn ensure_share(&mut self) -> bool {
        if self.share.is_some() {
            return true;
        }
        let server = self.share_server.trim().to_string();
        if server.is_empty() {
            self.share_status = "Kein Server eingetragen (Einstellungen → TEILEN)".to_string();
            return false;
        }
        let device = if self.share_device_draft.trim().is_empty() {
            default_device_name()
        } else {
            self.share_device_draft.trim().to_string()
        };
        match crate::share::ShareService::start(server, device) {
            Ok(svc) => {
                self.share = Some(svc);
                true
            }
            Err(e) => {
                self.error_msg = Some(format!("Share-Server-Dienst: {}", e));
                false
            }
        }
    }

    pub(in crate::app) fn share_cmd(&mut self, c: crate::share::ShareCmd) {
        if self.ensure_share() {
            if let Some(svc) = &self.share {
                svc.cmd(c);
            }
        }
    }

    /// Drain Quick Share discovery if some other entry point started it. The
    /// Share-Server connection panel does not start Quick Share.
    pub(in crate::app) fn drain_quickshare(&mut self) {
        if let Some(qs) = &self.quickshare {
            for list in qs.events.try_iter() {
                self.qs_devices = list;
            }
        }
    }

    pub(in crate::app) fn drain_share(&mut self) {
        let events: Vec<crate::share::ShareEvent> = match &self.share {
            Some(svc) => svc.events.try_iter().collect(),
            None => return,
        };
        for ev in events {
            use crate::share::ShareEvent as E;
            match ev {
                E::Status(s) => self.share_status = s,
                E::Error(e) => {
                    self.share_status = format!("Fehler: {}", e);
                    self.error_msg = Some(format!("Share-Server: {}", e));
                }
                E::Roster(r) => self.share_roster = r,
                E::Incoming { id, from, files } => {
                    self.share_incoming.push((id, from, files));
                    self.show_share = true;
                }
                E::Progress { done, total } => self.share_progress = Some((done, total)),
                E::Received { count, dir } => {
                    self.share_progress = None;
                    self.share_status = format!("✓ {} empfangen → {}", count, dir);
                    self.notice = Some((
                        format!("📥 {} Datei(en) empfangen", count),
                        std::time::Instant::now(),
                    ));
                }
                E::Sent { count } => {
                    self.share_progress = None;
                    self.share_status = format!("✓ {} gesendet", count);
                }
            }
        }
    }

    pub(in crate::app) fn share_export_config(&self) -> crate::share::ShareExportConfig {
        crate::share::ShareExportConfig {
            roots: self.share_exports.clone(),
            include_connections: self.share_include_connections,
        }
    }

    pub(in crate::app) fn add_share_export(&mut self, path: String, label: String) {
        let path = path.trim().replace('\\', "/");
        if path.is_empty() {
            return;
        }
        let label = if label.trim().is_empty() {
            path.trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("Freigabe")
                .to_string()
        } else {
            label.trim().to_string()
        };
        if self.share_exports.iter().any(|r| r.path == path) {
            return;
        }
        self.share_exports
            .push(crate::share::SharedRoot { label, path });
        self.share_cmd(crate::share::ShareCmd::SetExports(
            self.share_export_config(),
        ));
    }

    pub(in crate::app) fn open_share_peer(&mut self, peer: &crate::share::RemoteDevice) {
        let backend = match self
            .share
            .as_ref()
            .and_then(|svc| svc.backend_for_peer(&peer.fingerprint))
        {
            Some(be) => be,
            None => {
                self.error_msg = Some("Share-Server: keine aktive Peer-Sitzung".to_string());
                return;
            }
        };
        let label = if self.share_room {
            let room = if self.share_session_code.is_empty() {
                "Raum".to_string()
            } else {
                format!("Raum {}", self.share_session_code)
            };
            format!("Share: {} / {}", room, peer.device)
        } else {
            format!("Share: {}", peer.device)
        };
        self.remote = Some(crate::connect::RemoteState {
            backend: cache_remote(backend),
            label: label.clone(),
            agent_version: None,
            zip_return: None,
            sftp: None,
            account: None,
            endpoint_prefix: None,
        });
        self.net_conn = None;
        self.notice = Some((format!("Verbunden: {}", label), std::time::Instant::now()));
        self.start_scan(PathBuf::from("/"));
    }

    pub(in crate::app) fn ui_share(&mut self, ctx: &egui::Context) {
        let mut open = self.show_share;
        let mut pair_show = false;
        let mut pair_connect = false;
        let mut room_join = false;
        let mut leave = false;
        let mut add_export = false;
        let mut add_current = false;
        let mut add_all_drives = false;
        let mut pick_export = false;
        let mut remove_export: Option<usize> = None;
        let mut open_peer: Option<crate::share::RemoteDevice> = None;
        let mut exports_changed = false;

        let roster = self.share_roster.clone();
        let status = self.share_status.clone();
        let my_code = self.share_my_code.clone();
        let session_code = self.share_session_code.clone();
        let fingerprint = self
            .share
            .as_ref()
            .map(|s| s.fingerprint.clone())
            .unwrap_or_default();
        let exports = self.share_exports.clone();
        let server_missing = self.share_server.trim().is_empty();

        egui::Window::new("Verbinden via Share-Server")
            .open(&mut open)
            .resizable(true)
            .default_size([520.0, 560.0])
            .show(ctx, |ui| {
                if server_missing {
                    ui.colored_label(
                        Color32::from_rgb(255, 185, 120),
                        "Kein Share-Server eingetragen.",
                    );
                    ui.label("Einstellungen -> TEILEN: Server-Adresse (host:port) setzen.");
                    return;
                }
                if !fingerprint.is_empty() {
                    ui.label(
                        RichText::new(format!("Dieses Geraet: {}", fingerprint))
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                }

                ui.add_space(6.0);
                ui.label(
                    RichText::new("FREIGABEN")
                        .small()
                        .color(Color32::from_gray(140)),
                );
                if exports.is_empty() && !self.share_include_connections {
                    ui.colored_label(Color32::from_gray(140), "(nichts freigegeben)");
                }
                for (i, r) in exports.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} -> {}", r.label, r.path));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("x")
                                .on_hover_text("Freigabe entfernen")
                                .clicked()
                            {
                                remove_export = Some(i);
                            }
                        });
                    });
                }
                if ui
                    .checkbox(
                        &mut self.share_include_connections,
                        "Eigene gespeicherte Verbindungen freigeben (ohne Peer-Share-Rekursion)",
                    )
                    .changed()
                {
                    exports_changed = true;
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.share_export_label_draft)
                            .hint_text("Name")
                            .desired_width(120.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.share_export_path_draft)
                            .hint_text("Ordner, Laufwerk oder UNC")
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Aktuellen Ordner").clicked() {
                        add_current = true;
                    }
                    if ui.button("Waehlen...").clicked() {
                        pick_export = true;
                    }
                    if ui.button("Hinzufuegen").clicked() {
                        add_export = true;
                    }
                    if ui.button("Alle Laufwerke").clicked() {
                        add_all_drives = true;
                    }
                });

                ui.separator();
                ui.label(
                    RichText::new("DIREKT")
                        .small()
                        .color(Color32::from_gray(140)),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button("Code anzeigen")
                        .on_hover_text("Erzeugt einen Code; das andere Geraet gibt ihn ein")
                        .clicked()
                    {
                        pair_show = true;
                    }
                    if !my_code.is_empty() {
                        ui.label(RichText::new(&my_code).monospace().strong().size(18.0));
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.share_code_input)
                            .hint_text("Code")
                            .desired_width(160.0),
                    );
                    if ui.button("Direkt verbinden").clicked() {
                        pair_connect = true;
                    }
                });

                ui.add_space(8.0);
                ui.label(RichText::new("RAUM").small().color(Color32::from_gray(140)));
                ui.horizontal(|ui| {
                    if ui.button("Raum erstellen").clicked() {
                        room_join = true;
                    }
                    if ui.button("Raum beitreten").clicked() {
                        room_join = true;
                    }
                    if !session_code.is_empty() {
                        ui.label(RichText::new(format!("Raum/Code: {session_code}")).monospace());
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Geraete ({})", roster.len())).strong());
                    if !roster.is_empty() && ui.small_button("Verlassen").clicked() {
                        leave = true;
                    }
                });
                if roster.is_empty() {
                    ui.colored_label(Color32::from_gray(140), "(noch keine Geraete gefunden)");
                }
                for d in &roster {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} ({})", d.device, d.fingerprint));
                        if ui.button("Oeffnen").clicked() {
                            open_peer = Some(d.clone());
                        }
                    });
                }
                if !status.is_empty() {
                    ui.label(
                        RichText::new(&status)
                            .small()
                            .color(Color32::from_gray(150)),
                    );
                }
            });
        self.show_share = open;

        if let Some(i) = remove_export {
            if i < self.share_exports.len() {
                self.share_exports.remove(i);
                self.share_cmd(crate::share::ShareCmd::SetExports(
                    self.share_export_config(),
                ));
            }
        }
        if add_current && !self.root_path.is_empty() && self.remote.is_none() {
            let label = self
                .root_path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("Aktuell")
                .to_string();
            self.add_share_export(self.root_path.clone(), label);
        }
        if pick_export {
            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                let path = p.to_string_lossy().replace('\\', "/");
                self.share_export_path_draft = path.clone();
                self.share_export_label_draft = path
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Freigabe")
                    .to_string();
            }
        }
        if add_export {
            self.add_share_export(
                self.share_export_path_draft.clone(),
                self.share_export_label_draft.clone(),
            );
        }
        if add_all_drives {
            for d in self.drives.clone() {
                let label = d.trim_end_matches(['\\', '/']).to_string();
                self.add_share_export(d, label);
            }
        }
        if exports_changed {
            self.share_cmd(crate::share::ShareCmd::SetExports(
                self.share_export_config(),
            ));
        }

        if pair_show {
            let code = crate::share::gen_code();
            self.share_my_code = code.clone();
            self.share_session_code = code.clone();
            self.share_room = false;
            self.share_cmd(crate::share::ShareCmd::Pair {
                code,
                exports: self.share_export_config(),
            });
        }
        if pair_connect {
            let code = self.share_code_input.trim().to_string();
            if !code.is_empty() {
                self.share_my_code.clear();
                self.share_session_code = code.clone();
                self.share_room = false;
                self.share_cmd(crate::share::ShareCmd::Pair {
                    code,
                    exports: self.share_export_config(),
                });
            }
        }
        if room_join {
            let code = if self.share_code_input.trim().is_empty() {
                let c = crate::share::gen_code();
                self.share_my_code = c.clone();
                c
            } else {
                self.share_code_input.trim().to_string()
            };
            self.share_session_code = code.clone();
            self.share_room = true;
            self.share_cmd(crate::share::ShareCmd::JoinRoom {
                code,
                exports: self.share_export_config(),
            });
        }
        if leave {
            self.share_roster.clear();
            self.share_my_code.clear();
            self.share_session_code.clear();
            self.share_cmd(crate::share::ShareCmd::Leave);
        }
        if let Some(peer) = open_peer {
            self.open_share_peer(&peer);
        }
    }
}
