use super::*;

impl App {
    pub(super) fn generate_room_draft_code(&mut self) {
        match crate::share::ShareProfiles::new_room_code() {
            Ok(code) => self.share_room_draft_code = code,
            Err(error) => {
                self.share_room_draft_code.clear();
                self.error_msg = Some(format!("Raum-Code nicht sicher erzeugt: {error}"));
            }
        }
    }

    pub(super) fn ui_share_rooms(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Räume entdecken").id_salt("share_discover_rooms_v2")
            .show(ui, |ui| self.ui_share_discovery_rooms(ui));
        ui.separator();
        egui::CollapsingHeader::new("Raum erstellen").id_salt("share_create_room_v2").show(ui, |ui| {
        ui.label(
            RichText::new("RAUM ERSTELLEN")
                .small()
                .color(theme::muted(ui)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.share_room_create_name_input)
                    .desired_width(160.0)
                    .clip_text(true),
            );
            share_value_field(ui, &self.share_room_draft_code);
            if ui.button("Neuen Code").clicked() {
                self.generate_room_draft_code();
            }
            if ui.button("Code kopieren").clicked() {
                ui.ctx().copy_text(self.share_room_draft_code.clone());
            }
            if ui.button("Raum erstellen").clicked() {
                match crate::share::ShareProfiles::add_room_from_code_persisted(
                    Some(dirs_home().to_string_lossy().replace('\\', "/")),
                    &self.share_room_draft_code,
                    &self.share_room_create_name_input,
                ) {
                    Ok((profiles, _)) => {
                        self.share_profiles = profiles;
                        self.generate_room_draft_code();
                        let _ = self.configure_share_service();
                    }
                    Err(e) => self.error_msg = Some(e),
                }
            }
            if ui.button("Leeren").clicked() {
                self.share_room_create_name_input.clear();
            }
        });

        });
        ui.separator();
        egui::CollapsingHeader::new("Raum per Code beitreten").id_salt("share_join_room_v2").show(ui, |ui| {
        ui.label(
            RichText::new("RAUM BEITRETEN")
                .small()
                .color(theme::muted(ui)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.share_room_code_input)
                    .hint_text("SE-R3-...")
                    .desired_width(share_input_width(ui, 360.0))
                    .clip_text(true),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.share_room_name_input)
                    .hint_text("Name")
                    .desired_width(120.0),
            );
            if ui.button("Einfuegen").clicked() {
                self.notice = Some((
                    "Bitte mit Strg+V in das Code-Feld einfuegen".to_string(),
                    std::time::Instant::now(),
                ));
            }
            if ui.button("Beitreten").clicked() {
                match crate::share::ShareProfiles::add_room_from_code_persisted(
                    Some(dirs_home().to_string_lossy().replace('\\', "/")),
                    &self.share_room_code_input,
                    &self.share_room_name_input,
                ) {
                    Ok((profiles, _)) => {
                        self.share_profiles = profiles;
                        self.share_room_code_input.clear();
                        self.share_room_name_input.clear();
                        let _ = self.configure_share_service();
                    }
                    Err(e) => self.error_msg = Some(e),
                }
            }
            if ui.button("Leeren").clicked() {
                self.share_room_code_input.clear();
                self.share_room_name_input.clear();
            }
        });

        });
        ui.separator();
        ui.label(
            RichText::new("GESPEICHERTE RAEUME")
                .small()
                .color(theme::muted(ui)),
        );
        let mut remove_room: Option<String> = None;
        let mut open_target: Option<crate::share::PeerOpenTarget> = None;
        let mut pending_diag: Option<String> = None;
        let mut leave_room: Option<String> = None;
        let mut changed = false;
        let previous_profiles = self.share_profiles.clone();
        for room in &mut self.share_profiles.rooms {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(format!(
                        "{} [{}] Mitglieder: {}",
                        room.name,
                        room.status.label(),
                        room.members.len()
                    ))
                    .wrap(),
                );
                if ui.button("Oeffnen").clicked() {
                    room.status = crate::share::ShareStatus::Available;
                }
                if ui.button("Beitreten").clicked() {
                    room.auto_join = true;
                    changed = true;
                }
                if ui.button("Verlassen").clicked() {
                    leave_room = Some(room.room_id.clone());
                    room.auto_join = false;
                    room.status = crate::share::ShareStatus::Offline;
                    changed = true;
                }
                if ui.checkbox(&mut room.auto_join, "Auto").changed() {
                    changed = true;
                }
                if ui.button("Freigaben").clicked() {
                    self.share_export_scope = 2;
                    self.share_export_target_id = room.id.clone();
                    self.share_tab = 2;
                }
                if ui.button("Code kopieren").clicked() {
                    match crate::share::ShareProfiles::room_code_checked(room) {
                        Ok(Some(code)) => ui.ctx().copy_text(code),
                        Ok(None) => self.error_msg = Some("Raum-Secret fehlt".into()),
                        Err(error) => {
                            self.error_msg = Some(format!("Raum-Code lesen: {error}"));
                        }
                    }
                }
                if ui.button("Umbenennen").clicked() {
                    room.name = self.share_room_name_input.trim().to_string();
                    changed = true;
                }
                if ui.button("Entfernen").clicked() {
                    remove_room = Some(room.id.clone());
                }
            });
            for member in &mut room.members {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Label::new(format!(
                            "  {} [{}]",
                            member.device_name,
                            member.status.label()
                        ))
                        .wrap(),
                    );
                    if ui.button("Oeffnen").clicked() {
                        open_target = Some(crate::share::PeerOpenTarget::RoomDevice {
                            room_id: room.id.clone(),
                            device_id: member.device_id.clone(),
                        });
                    }
                    if ui.button("Diagnose").clicked() {
                        let presence = member
                            .presence
                            .as_ref()
                            .map(|p| {
                                format!(
                                    "candidates={:?}, expires_at={}",
                                    p.candidates, p.expires_at
                                )
                            })
                            .unwrap_or_else(|| "keine Presence".to_string());
                        pending_diag = Some(format!(
                            "Raum {} / {}: fp={}, status={}, {}\n",
                            room.name,
                            member.device_name,
                            member.fingerprint,
                            member.status.label(),
                            presence
                        ));
                    }
                    if ui.button("Fingerprint").clicked() {
                        ui.ctx().copy_text(member.fingerprint.clone());
                    }
                    if ui.checkbox(&mut member.blocked, "Blockieren").changed() {
                        changed = true;
                    }
                    if ui.button("Trust zuruecksetzen").clicked() {
                        member.presence = None;
                        member.status = crate::share::ShareStatus::Waiting;
                        changed = true;
                    }
                });
            }
        }
        if let Some(line) = pending_diag {
            self.append_share_diag(line);
            self.share_tab = 3;
        }
        let persisted = !changed || self.commit_share_profiles(previous_profiles);
        if let Some(room_id) = leave_room.filter(|_| persisted) {
            let _ = self.share_cmd(crate::share::ShareCmd::LeaveRoom { room_id });
        }
        if let Some(id) = remove_room.filter(|_| persisted) {
            self.remove_room_completely(&id);
        }
        if let Some(target) = open_target {
            self.open_share_target(target);
        }
    }
}
