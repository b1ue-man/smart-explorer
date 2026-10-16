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

    /// Page „Raum erstellen oder beitreten“: discovery by PIN, a new room
    /// with its code, joining by code.
    pub(super) fn ui_share_room_forms(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Räume entdecken und per PIN beitreten")
            .id_salt("share_discover_rooms_v2")
            .default_open(true)
            .show(ui, |ui| self.ui_share_discovery_rooms(ui));
        ui.separator();
        egui::CollapsingHeader::new("Raum erstellen")
            .id_salt("share_create_room_v2")
            .default_open(true)
            .show(ui, |ui| {
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
        egui::CollapsingHeader::new("Raum per Code beitreten")
            .id_salt("share_join_room_v2")
            .default_open(true)
            .show(ui, |ui| {
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
    }
}
