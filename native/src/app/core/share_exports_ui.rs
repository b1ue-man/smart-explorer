use super::*;

impl App {
    pub(super) fn selected_export_config(&self) -> crate::share::ShareExportConfig {
        match self.share_export_scope {
            2 => self
                .share_profiles
                .rooms
                .iter()
                .find(|r| r.id == self.share_export_target_id)
                .map(|r| r.exports.clone())
                .unwrap_or_else(|| self.share_profiles.default_direct_exports.clone()),
            _ => self.share_profiles.default_direct_exports.clone(),
        }
    }

    pub(super) fn set_selected_export_config(&mut self, cfg: crate::share::ShareExportConfig) {
        let previous_profiles = self.share_profiles.clone();
        match self.share_export_scope {
            2 => {
                if let Some(r) = self
                    .share_profiles
                    .rooms
                    .iter_mut()
                    .find(|r| r.id == self.share_export_target_id)
                {
                    r.exports = cfg;
                }
            }
            _ => self.share_profiles.default_direct_exports = cfg,
        }
        let _ = self.commit_share_profiles(previous_profiles);
    }

    pub(super) fn ui_share_exports(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.share_export_scope, 0, "Alle Direktkontakte");
            ui.selectable_value(&mut self.share_export_scope, 2, "Raum");
        });
        if self.share_export_scope == 2 {
            egui::ComboBox::from_label("Raum")
                .selected_text(selected_room_label(self))
                .show_ui(ui, |ui| {
                    for r in &self.share_profiles.rooms {
                        ui.selectable_value(
                            &mut self.share_export_target_id,
                            r.id.clone(),
                            &r.name,
                        );
                    }
                });
        }

        let mut cfg = self.selected_export_config();
        let mut remove: Option<usize> = None;
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;
        let mut changed = false;
        if ui
            .checkbox(
                &mut cfg.include_connections,
                "Eigene gespeicherte Verbindungen freigeben",
            )
            .changed()
        {
            changed = true;
        }
        ui.checkbox(
            &mut self.share_block_symlink_escape,
            "Symlinks ausserhalb der Freigabe blockieren",
        );
        ui.add_enabled(
            false,
            egui::Checkbox::new(&mut true, "Share-Server-Verbindungen ausschliessen"),
        );
        for (i, root) in cfg.roots.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::Label::new(format!("{} ->", root.label)).wrap());
                share_value_field(ui, &root.path);
                if ui.button("Test").clicked() {
                    self.append_share_diag(format!(
                        "Freigabe-Test {}: {}\n",
                        root.label,
                        if std::path::Path::new(&root.path).exists() {
                            "ok"
                        } else {
                            "nicht gefunden"
                        }
                    ));
                }
                if ui.button("Nach oben").clicked() && i > 0 {
                    move_up = Some(i);
                }
                if ui.button("Nach unten").clicked() && i + 1 < cfg.roots.len() {
                    move_down = Some(i);
                }
                if ui.button("Entfernen").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = move_up {
            cfg.roots.swap(i, i - 1);
            changed = true;
        }
        if let Some(i) = move_down {
            cfg.roots.swap(i, i + 1);
            changed = true;
        }
        if let Some(i) = remove {
            cfg.roots.remove(i);
            changed = true;
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.share_export_label_draft)
                    .hint_text("Name")
                    .desired_width(120.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.share_export_path_draft)
                    .hint_text("Ordner, Laufwerk oder UNC")
                    .desired_width(share_input_width(ui, 300.0))
                    .clip_text(true),
            );
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Ordner hinzufuegen").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.share_export_path_draft = p.to_string_lossy().replace('\\', "/");
                }
            }
            if ui.button("Aktuellen Ordner hinzufuegen").clicked()
                && self.remote.is_none()
                && !self.root_path.is_empty()
            {
                self.share_export_path_draft = self.root_path.clone();
            }
            if ui.button("Laufwerk hinzufuegen").clicked() {
                if let Some(d) = self.drives.first() {
                    self.share_export_path_draft = d.clone();
                }
            }
            if ui.button("Alle Laufwerke hinzufuegen").clicked() {
                for d in self.drives.clone() {
                    let label = d.trim_end_matches(['\\', '/']).to_string();
                    if !cfg.roots.iter().any(|r| r.path == d) {
                        cfg.roots.push(crate::share::SharedRoot { label, path: d });
                        changed = true;
                    }
                }
            }
            if ui.button("Gespeicherte Verbindung hinzufuegen").clicked() {
                cfg.include_connections = true;
                changed = true;
            }
            if ui.button("Alle gespeicherten Verbindungen").clicked() {
                cfg.include_connections = true;
                changed = true;
            }
            if ui.button("Hinzufuegen").clicked() {
                let path = self.share_export_path_draft.trim().replace('\\', "/");
                if !path.is_empty() && !cfg.roots.iter().any(|r| r.path == path) {
                    cfg.roots.push(crate::share::SharedRoot {
                        label: self.share_export_label_draft.trim().to_string(),
                        path,
                    });
                    changed = true;
                }
            }
            if ui.button("Alles entfernen").clicked() {
                cfg.roots.clear();
                changed = true;
            }
        });
        if changed {
            self.set_selected_export_config(cfg);
        }
    }
}
