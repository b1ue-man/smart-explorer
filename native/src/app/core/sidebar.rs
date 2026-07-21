use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        self.ui_sidebar_locations(ui);

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
        let mut open_share_target: Option<crate::share::PeerOpenTarget> = None;
        let mut mount_saved: Option<crate::creds::SavedConnection> = None;
        let mut mount_gdrive = false;
        let mut mount_peer: Option<(crate::share::PeerOpenTarget, String)> = None;
        let mount_supported = crate::mount::drive_mount_supported();

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
                    if mount_supported
                        && ui
                            .small_button("▣")
                            .on_hover_text("Google Drive als lokales Laufwerk einbinden")
                            .clicked()
                    {
                        mount_gdrive = true;
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
                    if mount_supported
                        && ui
                            .small_button("▣")
                            .on_hover_text("Als lokales Laufwerk einbinden")
                            .clicked()
                    {
                        mount_saved = Some(c.clone());
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

        if !self.share_profiles.direct_contacts.is_empty() || !self.share_profiles.rooms.is_empty()
        {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("SHARE DIREKT")
                        .small()
                        .color(Color32::from_gray(140)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("+")
                        .on_hover_text("Direktgeraet hinzufuegen")
                        .clicked()
                    {
                        self.show_share = true;
                        self.share_tab = 0;
                    }
                    if ui
                        .small_button("R")
                        .on_hover_text("Direktkontakte aktualisieren")
                        .clicked()
                    {
                        let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
                    }
                    if ui
                        .small_button("...")
                        .on_hover_text("Share-Server Verbindungen")
                        .clicked()
                    {
                        self.show_share = true;
                    }
                });
            });
            for c in self.share_profiles.direct_contacts.clone() {
                ui.horizontal(|ui| {
                    let target = crate::share::PeerOpenTarget::Direct {
                        contact_id: c.id.clone(),
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("{} [{}]", c.display_name, c.status.label()))
                                    .small(),
                            )
                            .frame(false),
                        )
                        .on_hover_text(format!(
                            "{} via Share-Server oeffnen",
                            c.expected_fingerprint
                        ))
                        .clicked()
                    {
                        open_share_target = Some(target.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if mount_supported
                            && ui
                                .small_button("▣")
                                .on_hover_text("Direktgeraet als lokales Laufwerk einbinden")
                                .clicked()
                        {
                            mount_peer = Some((target, c.display_name.clone()));
                        }
                    });
                });
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("SHARE RAEUME")
                        .small()
                        .color(Color32::from_gray(140)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("+")
                        .on_hover_text("Raum erstellen/beitreten")
                        .clicked()
                    {
                        self.show_share = true;
                        self.share_tab = 1;
                    }
                    if ui
                        .small_button("R")
                        .on_hover_text("Raeume aktualisieren")
                        .clicked()
                    {
                        let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
                    }
                });
            });
            for r in self.share_profiles.rooms.clone() {
                ui.label(
                    RichText::new(format!(
                        "{} [{}] ({})",
                        r.name,
                        r.status.label(),
                        r.members.len()
                    ))
                    .small()
                    .color(Color32::from_gray(150)),
                );
                for m in r.members {
                    ui.horizontal(|ui| {
                        let target = crate::share::PeerOpenTarget::RoomDevice {
                            room_id: r.id.clone(),
                            device_id: m.device_id.clone(),
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!(
                                        "  {} [{}]",
                                        m.device_name,
                                        m.status.label()
                                    ))
                                    .small(),
                                )
                                .frame(false),
                            )
                            .on_hover_text(format!("{} via Raum oeffnen", m.fingerprint))
                            .clicked()
                        {
                            open_share_target = Some(target.clone());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if mount_supported
                                && ui
                                    .small_button("▣")
                                    .on_hover_text("Raumgeraet als lokales Laufwerk einbinden")
                                    .clicked()
                            {
                                mount_peer =
                                    Some((target, format!("{} - {}", r.name, m.device_name)));
                            }
                        });
                    });
                }
            }
        }

        if disconnect {
            // Closing a ZIP returns to the folder it lives in; a real connection
            // returns to the landing page and releases every remote view row.
            let zip_return = self.remote.as_ref().and_then(|rs| rs.zip_return.clone());
            if let Some(parent) = zip_return {
                self.remote = None;
                self.net_conn = None;
                self.notice = Some(("Archiv geschlossen".to_string(), std::time::Instant::now()));
                self.start_scan(PathBuf::from(
                    parent.replace('/', std::path::MAIN_SEPARATOR_STR),
                ));
            } else {
                self.clear_disconnected_source_view();
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
            match crate::creds::remove_connection(&acc) {
                Ok(()) => {
                    self.saved_connections = crate::creds::load_connections();
                    self.notice = Some((
                        "Gespeicherte Verbindung entfernt".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    self.error_msg = Some(format!(
                        "Gespeicherte Verbindung konnte nicht entfernt werden: {error}"
                    ));
                }
            }
        }
        if let Some(c) = to_connect {
            self.connect_saved(&c);
        }
        if open_gdrive {
            self.open_gdrive_browse();
        }
        if disc_gdrive {
            match crate::cloud::disconnect(crate::cloud::Provider::GDrive) {
                Ok(()) => {
                    if gdrive_active {
                        self.clear_disconnected_source_view();
                    }
                    self.notice = Some((
                        "Google Drive getrennt".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Err(error) => {
                    self.error_msg = Some(format!("Google Drive trennen: {error}"));
                }
            }
        }
        if let Some(target) = open_share_target {
            self.open_share_target(target);
        }
        if let Some(connection) = mount_saved {
            self.offer_mount_saved(&connection);
        }
        if mount_gdrive {
            self.offer_mount_gdrive();
        }
        if let Some((target, label)) = mount_peer {
            self.offer_mount_peer(target, label);
        }
    }
}
