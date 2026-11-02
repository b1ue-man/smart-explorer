use super::*;

impl App {
    pub(super) fn ui_share_connections(
        &mut self,
        ui: &mut egui::Ui,
        config: &mut crate::share::ShareExportConfig,
    ) -> bool {
        ui.label("Gespeicherte Verbindungen einzeln freigeben");
        ui.label("Peers nutzen dabei deine gespeicherten Zugangsdaten. Gib nur die gewuenschten Verbindungen frei.");
        let (saved, readable) = match crate::creds::load_connections_checked() {
            Ok(saved) => (saved, true),
            Err(error) => {
                ui.label(format!("Gespeicherte Verbindungen nicht lesbar: {error}"));
                (Vec::new(), false)
            }
        };
        let mut changed = false;
        let wide = ui.available_width() >= egui::TextStyle::Body.resolve(ui.style()).size * 50.0;
        if wide {
            egui::Grid::new("shared_connection_rights")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Verbindung");
                    let headers = [
                        ui.strong("Freigeben").id,
                        ui.strong("Nur lesen").id,
                        ui.strong("Schreiben").id,
                    ];
                    ui.end_row();
                    for connection in &saved {
                        let account = connection.account();
                        let old = config.connection_access(&account);
                        let mut selected = old.is_some();
                        let mut access = old.unwrap_or(crate::share::ExportAccess::ReadOnly);
                        let label = ui
                            .add_sized([240.0, 0.0], egui::Label::new(connection.display()).wrap())
                            .on_hover_text(&account);
                        let selection = ui
                            .checkbox(&mut selected, "")
                            .labelled_by(label.id)
                            .labelled_by(headers[0])
                            .changed();
                        let read = ui
                            .add_enabled(
                                selected,
                                egui::RadioButton::new(
                                    access == crate::share::ExportAccess::ReadOnly,
                                    "",
                                ),
                            )
                            .labelled_by(label.id)
                            .labelled_by(headers[1]);
                        let write = ui
                            .add_enabled(
                                selected,
                                egui::RadioButton::new(
                                    access == crate::share::ExportAccess::ReadWrite,
                                    "",
                                ),
                            )
                            .labelled_by(label.id)
                            .labelled_by(headers[2]);
                        if read.clicked() {
                            access = crate::share::ExportAccess::ReadOnly;
                        }
                        if write.clicked() {
                            access = crate::share::ExportAccess::ReadWrite;
                        }
                        if selection || read.clicked() || write.clicked() {
                            match config.set_connection_access(&account, selected.then_some(access))
                            {
                                Ok(did_change) => changed |= did_change,
                                Err(error) => self.error_msg = Some(error),
                            }
                        }
                        ui.end_row();
                    }
                });
        } else {
            for connection in &saved {
                let account = connection.account();
                let old = config.connection_access(&account);
                let mut selected = old.is_some();
                let mut access = old.unwrap_or(crate::share::ExportAccess::ReadOnly);
                let mut edited = false;
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.add(egui::Label::new(connection.display()).wrap())
                        .on_hover_text(&account);
                    let mut form = crate::app::settings_form::Form::new(
                        ui,
                        &account,
                        &["Freigeben", "Zugriff"],
                    );
                    edited |= form.row_labelled(ui, "Freigeben", |ui, label| {
                        ui.checkbox(&mut selected, "Für diese Beziehung")
                            .labelled_by(label)
                            .changed()
                    });
                    form.row(ui, "Zugriff", |ui| {
                        ui.add_enabled_ui(selected, |ui| {
                            egui::ComboBox::from_id_salt(("shared_connection_access", &account))
                                .selected_text(if access == crate::share::ExportAccess::ReadOnly {
                                    "Nur lesen"
                                } else {
                                    "Lesen und Schreiben"
                                })
                                .show_ui(ui, |ui| {
                                    edited |= ui
                                        .selectable_value(
                                            &mut access,
                                            crate::share::ExportAccess::ReadOnly,
                                            "Nur lesen",
                                        )
                                        .changed();
                                    edited |= ui
                                        .selectable_value(
                                            &mut access,
                                            crate::share::ExportAccess::ReadWrite,
                                            "Lesen und Schreiben",
                                        )
                                        .changed();
                                });
                        });
                    });
                });
                if edited {
                    match config.set_connection_access(&account, selected.then_some(access)) {
                        Ok(did_change) => changed |= did_change,
                        Err(error) => self.error_msg = Some(error),
                    }
                }
            }
        }
        for connection in config.shared_connections.clone() {
            if saved
                .iter()
                .any(|saved| saved.account() == connection.account)
            {
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Gespeicherte Verbindung derzeit nicht verfuegbar:");
                share_value_field(ui, &connection.account);
                if ui.button("Freigabe entfernen").clicked() {
                    match config.set_connection_access(&connection.account, None) {
                        Ok(did_change) => changed |= did_change,
                        Err(error) => self.error_msg = Some(error),
                    }
                }
            });
        }
        if saved.is_empty() && readable {
            ui.label("Keine gespeicherten Verbindungen.");
        }
        changed
    }
}
