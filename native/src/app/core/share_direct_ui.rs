use super::*;

impl App {
    pub(super) fn ui_share_direct(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Meine Geräte");
        if self.share_profiles.direct_contacts.is_empty() {
            ui.label("Noch kein Gerät verbunden. Ein Gerät finden oder seinen Direkt-Code hinzufügen.");
        }
        self.ui_share_saved_devices(ui);
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Gerät per Direkt-Code hinzufügen")
            .id_salt("share_add_device_v2").show(ui, |ui| self.ui_share_add_device(ui));
        egui::CollapsingHeader::new("Geräte entdecken")
            .id_salt("share_discover_v2").default_open(self.share_profiles.direct_contacts.is_empty())
            .show(ui, |ui| self.ui_share_discovery_direct(ui));
        ui.separator();
        lifecycle_ui::ui_lifecycle(self, ui);
        egui::CollapsingHeader::new("Dieses Gerät & Direkt-Code")
            .id_salt("share_this_device_v2").show(ui, |ui| self.ui_share_identity_controls(ui));
        ui.separator();
        egui::CollapsingHeader::new(format!(
            "Quick Share (LAN) — {} gefunden",
            self.qs_devices.len()
        ))
        .id_salt("quickshare_devices")
        .show(ui, |ui| self.ui_quickshare_devices(ui));
    }

    fn ui_share_identity_controls(&mut self, ui: &mut egui::Ui) {
        let local_identity = self
            .share_identity
            .as_ref()
            .map(|identity| (identity.direct_code(), identity.fingerprint.clone()));
        ui.label(
            RichText::new("Dieses Gerät")
                .small()
                .color(theme::muted(ui)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Direkt-Code:");
            if let Some((direct_code, fingerprint)) = &local_identity {
                share_value_field(ui, direct_code);
                if ui.button("Code kopieren").clicked() {
                    ui.ctx().copy_text(direct_code.clone());
                }
                if ui.button("Fingerprint kopieren").clicked() {
                    ui.ctx().copy_text(fingerprint.clone());
                }
            } else {
                ui.colored_label(
                    theme::danger(ui),
                    self.share_identity_error
                        .as_deref()
                        .unwrap_or("Share-Identitaet nicht verfuegbar"),
                );
            }
            if ui.button("Freigaben fuer diesen Code").clicked() {
                self.share_export_scope = 0;
                self.share_export_target_id.clear();
                self.share_tab = 2;
            }
        });
        ui.label(format!(
            "Freigegeben: {}",
            export_summary(&self.share_profiles.default_direct_exports)
        ));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Name aendern").clicked() {
                match self.share_identity.as_mut() {
                    Some(identity) => {
                        match identity.set_device_name(self.share_device_draft.clone()) {
                            Ok(()) => {
                                let _ = self.configure_share_service();
                            }
                            Err(error) => {
                                self.error_msg =
                                    Some(format!("Share-Geraetename nicht gespeichert: {error}"));
                            }
                        }
                    }
                    None => {
                        self.error_msg = Some("Share-Identitaet nicht verfuegbar".into());
                    }
                }
            }
            if ui.button("Online schalten").clicked() {
                let _ = self.ensure_share();
                let _ = self.share_cmd(crate::share::ShareCmd::SetDirectOnline { online: true });
            }
            if ui.button("Offline schalten").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::SetDirectOnline { online: false });
            }
            if ui.button("Code neu generieren").clicked() {
                self.share_regenerate_direct_confirm = true;
            }
        });
        if self.share_regenerate_direct_confirm {
            ui.colored_label(
                theme::warning(ui),
                "Neuer Code invalidiert alte Direktkontakte zu diesem Geraet.",
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("Wirklich neu generieren").clicked() {
                    identity_rotation::rotate(self);
                }
                if ui.button("Abbrechen").clicked() {
                    self.share_regenerate_direct_confirm = false;
                }
            });
        }

    }

    fn ui_share_add_device(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Direkt-Code")
                .small()
                .color(theme::muted(ui)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.share_direct_code_input)
                    .hint_text("SE-D3-...")
                    .desired_width(share_input_width(ui, 360.0))
                    .clip_text(true),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.share_direct_name_input)
                    .hint_text("Name")
                    .desired_width(140.0),
            );
            if ui.button("Einfuegen").clicked() {
                self.notice = Some((
                    "Bitte mit Strg+V in das Code-Feld einfuegen".to_string(),
                    std::time::Instant::now(),
                ));
            }
            if ui.button("Hinzufuegen").clicked() {
                match crate::share::ShareProfiles::add_direct_from_code_persisted(
                    Some(dirs_home().to_string_lossy().replace('\\', "/")),
                    &self.share_direct_code_input,
                    &self.share_direct_name_input,
                ) {
                    Ok((profiles, id)) => {
                        self.share_profiles = profiles;
                        self.share_direct_code_input.clear();
                        self.share_direct_name_input.clear();
                        let _ = lifecycle_ui::queue_contact(self, &id);
                    }
                    Err(e) => self.error_msg = Some(e),
                }
            }
            if ui.button("Leeren").clicked() {
                self.share_direct_code_input.clear();
                self.share_direct_name_input.clear();
            }
        });

    }

    fn ui_share_saved_devices(&mut self, ui: &mut egui::Ui) {
        let mut remove: Option<String> = None;
        let mut open_target: Option<crate::share::PeerOpenTarget> = None;
        let mut request_direct: Option<String> = None;
        let mut pending_diag: Option<String> = None;
        let mut changed = false;
        let previous_profiles = self.share_profiles.clone();
        for c in &mut self.share_profiles.direct_contacts {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(format!(
                        "{} [{} / {}]",
                        c.display_name,
                        c.status.label(),
                        c.access_state.label()
                    ))
                    .wrap(),
                );
                if ui.button("Öffnen").clicked() {
                    open_target = Some(crate::share::PeerOpenTarget::Direct {
                        contact_id: c.id.clone(),
                    });
                }
                if ui
                    .add_enabled(
                        c.access_state != crate::share::DirectAccessState::Accepted,
                        egui::Button::new("Zugriff anfragen"),
                    )
                    .clicked()
                {
                    request_direct = Some(c.id.clone());
                }
                ui.menu_button("Verwalten", |ui| {
                if ui.checkbox(&mut c.auto_connect, "Automatisch verbinden").changed() {
                    changed = true;
                }
                if ui.checkbox(&mut c.auto_open, "Automatisch öffnen").changed() {
                    changed = true;
                }
                if ui.button("Diagnose").clicked() {
                    let presence = c
                        .presence
                        .as_ref()
                        .map(|p| {
                            format!(
                                "node={}, relay={}, candidates={:?}, expires_at={}",
                                p.node_id, p.relay_url, p.candidates, p.expires_at
                            )
                        })
                        .unwrap_or_else(|| "keine Presence".to_string());
                    pending_diag = Some(format!(
                        "Direct {}: lookup={}, fp={}, status={}, {}\n",
                        c.display_name,
                        c.lookup_id,
                        c.expected_fingerprint,
                        c.status.label(),
                        presence
                    ));
                }
                if ui.button("Fingerprint").clicked() {
                    ui.ctx().copy_text(c.expected_fingerprint.clone());
                }
                if ui.button("Vertrauen zurücksetzen").clicked() {
                    c.remote_device_id = None;
                    c.remote_public_key = None;
                    c.presence = None;
                    c.status = crate::share::ShareStatus::Waiting;
                    changed = true;
                }
                if ui.button("Entfernen").clicked() {
                    remove = Some(c.id.clone());
                }
                });
            });
        }
        if let Some(line) = pending_diag {
            self.append_share_diag(line);
            self.share_tab = 3;
        }
        let persisted = !changed || self.commit_share_profiles(previous_profiles);
        if let Some(id) = remove.filter(|_| persisted) {
            self.remove_direct_peer_completely(&id);
        }
        if let Some(contact_id) = request_direct.filter(|_| persisted) {
            let _ = lifecycle_ui::queue_contact(self, &contact_id);
        }
        if let Some(target) = open_target {
            self.open_share_target(target);
        }

    }
}
