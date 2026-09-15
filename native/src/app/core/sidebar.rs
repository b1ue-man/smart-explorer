use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        self.ui_sidebar_locations(ui);
        theme::section(ui, "Verbindungen");
        let mut disconnect = false;
        let mut activate_agent = false;
        let mut remove_agent = false;
        let agent_activating = self.agent_activate_rx.is_some();
        let mut to_connect = None;
        let mut to_remove = None;
        let mut open_gdrive = false;
        let mut disc_gdrive = false;
        let mut open_share_target = None;
        let mut mount_saved = None;
        let mut mount_gdrive = false;
        let mut mount_peer = None;
        let mount_supported = crate::mount::drive_mount_supported();

        if let Some(remote) = &self.remote {
            ui.horizontal(|ui| {
                sidebar_button(ui, &remote.label, true)
                    .on_hover_text(format!("Aktive Verbindung: {}", remote.label));
                ui.menu_button("⋯", |ui| {
                    if ui.button("Verbindung trennen").clicked() { disconnect = true; ui.close_menu(); }
                    if let Some(version) = &remote.agent_version {
                        ui.label(format!("Remote-Agent {version}"));
                        if remote.sftp.is_some() && ui.button("Agent entfernen").clicked() {
                            remove_agent = true; ui.close_menu();
                        }
                    } else if remote.sftp.is_some() {
                        if agent_activating { ui.spinner(); ui.label("Agent wird aktiviert…"); }
                        else if ui.button("Remote-Agent aktivieren").clicked() {
                            activate_agent = true; ui.close_menu();
                        }
                    }
                });
            });
        } else if self.net_conn.is_some() {
            ui.horizontal(|ui| {
                sidebar_button(ui, "Netzlaufwerk", true);
                if ui.small_button("⏏").on_hover_text("Verbindung trennen").clicked() { disconnect = true; }
            });
        }
        let gdrive_active = self.remote.as_ref().is_some_and(|remote| remote.backend.scheme() == crate::vfs::Scheme::GDrive);
        if crate::cloud::is_connected(crate::cloud::Provider::GDrive) {
            ui.horizontal(|ui| {
                if sidebar_button(ui, "Google Drive", gdrive_active).clicked() { open_gdrive = true; }
                ui.menu_button("⋯", |ui| {
                    if mount_supported && ui.button("Als Laufwerk einbinden…").clicked() { mount_gdrive = true; ui.close_menu(); }
                    if ui.button("Google Drive trennen").clicked() { disc_gdrive = true; ui.close_menu(); }
                });
            });
        }
        let conns: Vec<_> = self.saved_connections.iter().rev().cloned().collect();
        for connection in conns.iter().take(SIDEBAR_CONN_CAP) {
            ui.horizontal(|ui| {
                if sidebar_button(ui, &connection.display(), false).on_hover_text(connection.to_target()).clicked() {
                    to_connect = Some(connection.clone());
                }
                ui.menu_button("⋯", |ui| {
                    if mount_supported && ui.button("Als Laufwerk einbinden…").clicked() {
                        mount_saved = Some(connection.clone()); ui.close_menu();
                    }
                    if ui.button("Verbindung entfernen…").clicked() {
                        to_remove = Some(connection.account()); ui.close_menu();
                    }
                });
            });
        }
        if conns.len() > SIDEBAR_CONN_CAP {
            ui.small(format!("Weitere Verbindungen im Menü oben ({})", conns.len() - SIDEBAR_CONN_CAP));
        }
        if ui.add(egui::Button::new("+ Verbindung hinzufügen").frame(false)).clicked() {
            self.connect_form = crate::connect::ConnectForm::default();
            self.show_connect = true;
        }

        if !self.share_profiles.direct_contacts.is_empty() {
            theme::section(ui, "Geräte");
            for contact in self.share_profiles.direct_contacts.clone().into_iter().take(5) {
                ui.horizontal(|ui| {
                    let target = crate::share::PeerOpenTarget::Direct { contact_id: contact.id.clone() };
                    if sidebar_button(ui, &contact.display_name, false).on_hover_text(format!("{} · {}", contact.status.label(), contact.expected_fingerprint)).clicked() {
                        open_share_target = Some(target.clone());
                    }
                    ui.menu_button("⋯", |ui| {
                        ui.label(contact.status.label());
                        if mount_supported && ui.button("Als Laufwerk einbinden…").clicked() {
                            mount_peer = Some((target, contact.display_name.clone())); ui.close_menu();
                        }
                    });
                });
            }
            if ui.add(egui::Button::new("Alle Geräte verwalten…").frame(false)).clicked() {
                self.show_share = true; self.share_tab = 0;
            }
        }
        if !self.share_profiles.rooms.is_empty() {
            egui::CollapsingHeader::new("Räume").id_salt("sidebar_rooms_v2").show(ui, |ui| {
                for room in self.share_profiles.rooms.clone() {
                    egui::CollapsingHeader::new(&room.name).id_salt(("sidebar_room", &room.id)).show(ui, |ui| {
                        for member in &room.members {
                            ui.horizontal(|ui| {
                                let target = crate::share::PeerOpenTarget::RoomDevice { room_id: room.id.clone(), device_id: member.device_id.clone() };
                                if sidebar_button(ui, &member.device_name, false).on_hover_text(member.status.label()).clicked() {
                                    open_share_target = Some(target.clone());
                                }
                                ui.menu_button("⋯", |ui| {
                                    if mount_supported && ui.button("Als Laufwerk einbinden…").clicked() {
                                        mount_peer = Some((target, format!("{} – {}", room.name, member.device_name))); ui.close_menu();
                                    }
                                });
                            });
                        }
                    });
                }
                if ui.button("Räume verwalten…").clicked() { self.show_share = true; self.share_tab = 1; }
            });
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
            self.remove_saved_connection_completely(&acc);
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

fn sidebar_button(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    ui.add_sized([(ui.available_width() - 42.0).max(40.0), 30.0],
        egui::Button::new(label).frame(false).selected(selected).truncate())
}
