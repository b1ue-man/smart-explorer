use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("Smart Explorer");
        ui.add_space(4.0);
        if ui
            .selectable_label(
                self.root_path.is_empty() && self.remote.is_none() && self.net_conn.is_none(),
                "Startseite",
            )
            .clicked()
        {
            self.navigate_to_landing_page();
        }
        ui.add_space(6.0);

        // Folder search now lives in the combo-field at the top (Ctrl+F): type
        // to filter the list, with global folder jumps offered in its dropdown.
        ui.label(
            RichText::new("Ordnersuche → Suchleiste oben (Ctrl+F)")
                .small()
                .color(Color32::from_gray(140)),
        );

        ui.horizontal(|ui| {
            if self.index_building {
                ui.colored_label(
                    Color32::from_gray(140),
                    format!("⟳ Indizieren… {} Ordner", self.index_progress),
                );
                if ui.small_button("Stop").clicked() {
                    self.cancel_index_build();
                }
            } else if self.folder_index.is_empty() {
                ui.colored_label(Color32::from_gray(140), "Kein Index");
                if ui
                    .small_button("Bauen")
                    .on_hover_text("Scannt alle Laufwerke einmalig nach Ordnern (etwa 30-90s)")
                    .clicked()
                {
                    self.start_index_build();
                }
            } else {
                let count = self.folder_index.len();
                ui.colored_label(
                    Color32::from_gray(140),
                    format!(
                        "Index: {} Ordner",
                        count.to_string().chars().rev().enumerate().fold(
                            String::new(),
                            |acc, (i, c)| {
                                if i > 0 && i % 3 == 0 {
                                    format!("{}.{}", c, acc)
                                } else {
                                    format!("{}{}", c, acc)
                                }
                            }
                        )
                    ),
                );
                if ui
                    .small_button("⟳")
                    .on_hover_text("Index aktualisieren")
                    .clicked()
                {
                    self.start_index_build();
                }
            }
        });

        ui.add_space(8.0);

        // ─── Favorites (starred folders) ───────────────────────────────
        if !self.favorites.is_empty() {
            ui.label(
                RichText::new("★ FAVORITEN")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            let favs = self.favorites.clone();
            let mut nav: Option<String> = None;
            let mut unstar: Option<String> = None;
            for f in &favs {
                ui.horizontal(|ui| {
                    let label = {
                        let base = f.trim_end_matches('/').rsplit('/').next().unwrap_or(f);
                        if base.is_empty() {
                            f.as_str()
                        } else {
                            base
                        }
                    };
                    if ui
                        .selectable_label(self.location_key(&self.root_path) == *f, label)
                        .on_hover_text(f)
                        .clicked()
                    {
                        nav = Some(f.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("✕")
                            .on_hover_text("Aus Favoriten entfernen")
                            .clicked()
                        {
                            unstar = Some(f.clone());
                        }
                    });
                });
            }
            if let Some(p) = nav {
                self.navigate_to_location(&p);
            }
            if let Some(p) = unstar {
                self.toggle_favorite(&p);
            }
            ui.add_space(8.0);
        }

        ui.label(
            RichText::new("SCHNELLZUGRIFF")
                .small()
                .color(Color32::from_gray(140)),
        );
        let home = self.home.clone();
        for (label, sub) in [
            ("Home", ""),
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Downloads", "Downloads"),
            ("Pictures", "Pictures"),
            ("Music", "Music"),
            ("Videos", "Videos"),
        ] {
            let p = if sub.is_empty() {
                home.clone()
            } else {
                home.join(sub)
            };
            if ui
                .selectable_label(
                    self.root_path == p.to_string_lossy().replace('\\', "/"),
                    label,
                )
                .on_hover_text(p.to_string_lossy())
                .clicked()
            {
                self.start_scan(p);
            }
        }

        if !self.drive_info.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new("LAUFWERKE")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            let infos = self.drive_info.clone();
            for (d, free, total) in infos {
                if ui
                    .selectable_label(self.root_path == d.replace('\\', "/"), &d)
                    .clicked()
                {
                    self.start_scan(PathBuf::from(&d));
                }
                if total > 0 {
                    let used = total.saturating_sub(free);
                    let frac = used as f32 / total as f32;
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(150.0)
                            .desired_height(6.0),
                    )
                    .on_hover_text(format!(
                        "{} frei von {}",
                        format_bytes(free),
                        format_bytes(total)
                    ));
                }
            }
        }

        if !self.recent.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new("ZULETZT")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            let recent = self.recent.clone();
            for r in recent {
                let label = r.rsplit('/').next().unwrap_or(&r).to_string();
                let label = if label.is_empty() { r.clone() } else { label };
                if ui
                    .selectable_label(self.root_path == r, &label)
                    .on_hover_text(&r)
                    .clicked()
                {
                    self.start_scan(PathBuf::from(r.replace('/', std::path::MAIN_SEPARATOR_STR)));
                }
            }
        }

        // ─── Remote connections (set-up-once; freshest pinned here) ─────
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("VERBINDUNGEN")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("＋")
                    .on_hover_text("Neue Verbindung (SFTP / FTP / FTPS / Netzlaufwerk)")
                    .clicked()
                {
                    self.connect_form = crate::connect::ConnectForm::default();
                    self.show_connect = true;
                }
            });
        });

        let mut disconnect = false;
        let mut activate_agent = false;
        let mut remove_agent = false;
        let agent_activating = self.agent_activate_rx.is_some();
        let mut to_connect: Option<crate::creds::SavedConnection> = None;
        let mut to_remove: Option<String> = None;
        let mut open_gdrive = false;
        let mut disc_gdrive = false;

        // Active connection indicator + one-click disconnect.
        if let Some(rs) = &self.remote {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(120, 200, 255), format!("● {}", rs.label));
                // SSH remote agent: show it's active, or offer to activate it on
                // THIS already-connected session (no reconnect, #24).
                if let Some(ver) = &rs.agent_version {
                    ui.colored_label(Color32::from_rgb(120, 230, 140), "⚡ Agent")
                        .on_hover_text(format!(
                            "Remote-Agent aktiv (v{ver}) — Erkundung/Analyse/Transfers laufen serverseitig"
                        ));
                    if rs.sftp.is_some()
                        && ui
                            .small_button("✖")
                            .on_hover_text(
                                "Remote-Agent entfernen — löscht ~/.cache/smart-explorer auf dem \
                                 Server und schaltet diese Verbindung zurück auf reines SFTP.",
                            )
                            .clicked()
                    {
                        remove_agent = true;
                    }
                } else if rs.sftp.is_some() {
                    if agent_activating {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(RichText::new("Agent…").small().color(Color32::from_gray(150)));
                    } else if ui
                        .small_button("⚡ Agent aktivieren")
                        .on_hover_text(
                            "Den Remote-Agent jetzt auf dieser Verbindung ausrollen — \
                             Listing/Analyse laufen dann serverseitig. Wird für diese \
                             Verbindung gemerkt. Fällt bei Problemen auf normales SFTP zurück.",
                        )
                        .clicked()
                    {
                        activate_agent = true;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("⏏").on_hover_text("Verbindung trennen").clicked() {
                        disconnect = true;
                    }
                });
            });
        } else if self.net_conn.is_some() {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(120, 200, 255), "● Netzlaufwerk");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("⏏")
                        .on_hover_text("Verbindung trennen")
                        .clicked()
                    {
                        disconnect = true;
                    }
                });
            });
        }

        // Pinned Google Drive — stays here whenever Drive is connected, even
        // with no tab open on it (click to browse, × to disconnect).
        let gdrive_active = self
            .remote
            .as_ref()
            .map(|rs| rs.backend.scheme() == crate::vfs::Scheme::GDrive)
            .unwrap_or(false);
        if crate::cloud::is_connected(crate::cloud::Provider::GDrive) {
            ui.horizontal(|ui| {
                let txt = RichText::new("☁ Google Drive").small();
                let txt = if gdrive_active {
                    txt.color(Color32::from_rgb(120, 200, 255))
                } else {
                    txt
                };
                if ui
                    .add(egui::Button::new(txt).frame(false))
                    .on_hover_text("Google Drive durchsuchen")
                    .clicked()
                {
                    open_gdrive = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("×")
                        .on_hover_text("Google Drive trennen")
                        .clicked()
                    {
                        disc_gdrive = true;
                    }
                });
            });
        }

        // Saved connections, newest first, capped — click to connect, × forget.
        let conns: Vec<crate::creds::SavedConnection> =
            self.saved_connections.iter().rev().cloned().collect();
        if conns.is_empty() {
            ui.colored_label(Color32::from_gray(120), "(noch keine gespeichert)");
        }
        for c in conns.iter().take(SIDEBAR_CONN_CAP) {
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new(format!("🖧 {}", c.display())).small())
                            .frame(false),
                    )
                    .on_hover_text(c.to_target())
                    .clicked()
                {
                    to_connect = Some(c.clone());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Entfernen").clicked() {
                        to_remove = Some(c.account());
                    }
                });
            });
        }
        if conns.len() > SIDEBAR_CONN_CAP {
            ui.colored_label(
                Color32::from_gray(120),
                format!(
                    "+{} ältere im Menü „Verbindung“",
                    conns.len() - SIDEBAR_CONN_CAP
                ),
            );
        }

        if disconnect {
            // Closing a ZIP returns to the folder it lives in; a real connection
            // just drops (entries clear on the next navigation).
            let zip_return = self.remote.as_ref().and_then(|rs| rs.zip_return.clone());
            self.remote = None;
            self.net_conn = None;
            if let Some(parent) = zip_return {
                self.notice = Some(("Archiv geschlossen".to_string(), std::time::Instant::now()));
                self.start_scan(PathBuf::from(
                    parent.replace('/', std::path::MAIN_SEPARATOR_STR),
                ));
            } else {
                self.notice = Some(("Verbindung getrennt".to_string(), std::time::Instant::now()));
            }
        }
        if activate_agent {
            self.start_agent_activation();
        }
        if remove_agent {
            self.remove_agent_now();
        }
        if self.agent_activate_rx.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        if let Some(acc) = to_remove {
            let _ = crate::creds::remove_connection(&acc);
            self.saved_connections = crate::creds::load_connections();
        }
        if let Some(c) = to_connect {
            self.connect_saved(&c);
        }
        if open_gdrive {
            self.open_gdrive_browse();
        }
        if disc_gdrive {
            crate::cloud::disconnect(crate::cloud::Provider::GDrive);
            if gdrive_active {
                self.remote = None;
            }
            self.notice = Some((
                "Google Drive getrennt".to_string(),
                std::time::Instant::now(),
            ));
        }
    }
}
