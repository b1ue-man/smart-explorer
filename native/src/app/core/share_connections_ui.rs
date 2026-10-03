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
        for connection in &saved {
            let account = connection.account();
            let old = config.connection_access(&account);
            let mut selected = old.is_some();
            let mut access = old.unwrap_or(crate::share::ExportAccess::ReadOnly);
            ui.horizontal_wrapped(|ui| {
                let selection = ui.checkbox(&mut selected, connection.display()).changed();
                share_value_field(ui, &account);
                let rights = ui
                    .add_enabled_ui(selected, |ui| {
                        ui.selectable_value(
                            &mut access,
                            crate::share::ExportAccess::ReadOnly,
                            "Nur lesen",
                        )
                        .changed()
                            | ui.selectable_value(
                                &mut access,
                                crate::share::ExportAccess::ReadWrite,
                                "Lesen und schreiben",
                            )
                            .changed()
                    })
                    .inner;
                if selection || rights {
                    match config.set_connection_access(&account, selected.then_some(access)) {
                        Ok(did_change) => changed |= did_change,
                        Err(error) => self.error_msg = Some(error),
                    }
                }
            });
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
