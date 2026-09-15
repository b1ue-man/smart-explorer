use super::prelude::*;
use super::*;

impl App {
    pub(super) fn toolbar_commands_width(ui: &egui::Ui, wide: bool) -> f32 {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let labels = ["Rekursiv", "Neu", "Verbindung", "Sync", "Einstellungen", "Ansicht", "»"];
        let mut width = 16.0; // separator and rounding to physical pixels
        for label in labels.into_iter().chain(wide.then_some("Share-Server")) {
            width += ui.fonts(|fonts| fonts.layout_no_wrap(label.into(), font.clone(), Color32::WHITE).size().x)
                + 2.0 * ui.spacing().button_padding.x + ui.spacing().item_spacing.x;
        }
        width
    }

    pub(in crate::app) fn ui_commandbar(&mut self, ui: &mut egui::Ui, wide: bool) {
        if ui.toggle_value(&mut self.recursive, "Rekursiv")
            .on_hover_text("Unterordner durchsuchen (Ctrl+R)").changed() && !self.root_path.is_empty() {
            self.rescan();
        }
        ui.separator();
        ui.add_enabled_ui(!self.root_path.is_empty(), |ui| self.ui_new_menu(ui));
        ui.menu_button("Verbindung", |ui| {
            ui.set_width(300.0);
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| self.ui_menu_connect(ui));
            if crate::mount::drive_mount_supported() && ui.button("Remote-Laufwerke…").clicked() {
                self.open_mount_manager();
                ui.close_menu();
            }
        });
        ui.menu_button("Sync", |ui| {
            ui.set_width(320.0);
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| self.ui_menu_sync(ui));
        });
        if ui.button("Einstellungen").clicked() { self.settings.open = !self.settings.open; }
        if wide && ui.selectable_label(self.show_share, "Share-Server").clicked() {
            self.show_share = !self.show_share;
        }
        let view = ui.menu_button("Ansicht", |ui| self.ui_view_menu(ui));
        self.accel_push('S', view.response.rect, AccelAct::Split);
        ui.menu_button("»", |ui| {
            if !wide {
                if ui.selectable_label(self.show_share, "Share-Server").clicked() {
                    self.show_share = !self.show_share;
                    ui.close_menu();
                }
                ui.separator();
            }
            self.ui_file_menu(ui);
        }).response.on_hover_text("Weitere Befehle");
    }

    fn ui_new_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Neu", |ui| {
            if ui.button("Ordner").clicked() {
                self.create_new_folder();
                ui.close_menu();
            }
            ui.separator();
            for (label, base, ext) in [
                ("Textdatei", "Neue Textdatei", "txt"),
                ("Markdown", "Neue Notiz", "md"),
                ("CSV-Tabelle", "Neue Tabelle", "csv"),
                ("JSON", "Neue Datei", "json"),
                ("HTML", "Neue Seite", "html"),
                ("Rust-Datei", "Neue Datei", "rs"),
            ] {
                if ui.button(label).clicked() {
                    self.create_new_file(base, ext);
                    ui.close_menu();
                }
            }
        });
    }

    fn ui_file_menu(&mut self, ui: &mut egui::Ui) {
        let selected = !self.selection.is_empty();
        for (label, cut) in [("Kopieren   Ctrl+C", false), ("Ausschneiden   Ctrl+X", true)] {
            if ui.add_enabled(selected, egui::Button::new(label)).clicked() {
                self.clipboard_copy_files(cut);
                ui.close_menu();
            }
        }
        if ui.add_enabled(!self.root_path.is_empty(), egui::Button::new("Einfügen   Ctrl+V")).clicked() {
            self.clipboard_paste_files();
            ui.close_menu();
        }
        ui.separator();
        let disposition = self.remote.as_ref().map(|remote| remote.backend.delete_disposition());
        let (label, tip) = match disposition {
            Some(crate::vfs::DeleteDisposition::Permanent) => ("Löschen…", "Server löscht endgültig; Bestätigung erforderlich"),
            Some(crate::vfs::DeleteDisposition::Unsupported) => ("Löschen", "Diese Quelle ist schreibgeschützt"),
            _ => ("In Papierkorb", "Ausgewählte Einträge in den Papierkorb verschieben"),
        };
        if ui.add_enabled(selected && disposition != Some(crate::vfs::DeleteDisposition::Unsupported),
            egui::Button::new(label)).on_hover_text(tip).clicked() {
            self.trash_selected();
            ui.close_menu();
        }
    }

    fn ui_view_menu(&mut self, ui: &mut egui::Ui) {
        if ui.selectable_label(self.split, "Zwei Bereiche   F6").clicked() {
            self.toggle_split();
            ui.close_menu();
        }
        if ui.checkbox(&mut self.show_summary, "Zusammenfassung").changed() {
            self.save_ui_state();
        }
        if ui.checkbox(&mut self.appearance.detailed_columns, "Zusätzliche Spalten").on_hover_text("Pfad, Erstelldatum und Verzeichnistiefe anzeigen").changed() {
            self.save_ui_state();
        }
        if ui.checkbox(&mut self.appearance.compact, "Kompakte Dateizeilen").changed() {
            self.save_ui_state();
        }
        ui.separator();
        if ui.checkbox(&mut self.dirs_first, "Ordner zuerst").changed() {
            if !self.root_path.is_empty() {
                self.dir_sort.insert(self.location_key(&self.root_path), self.dirs_first);
                if let Err(error) = save_dir_sort(&self.dir_sort) {
                    self.error_msg = Some(format!("Ordnersortierung speichern: {error}"));
                }
            }
            self.recompute_view();
        }
        if ui.selectable_label(self.show_analytics, "Speicheranalyse…").clicked() {
            self.show_analytics = !self.show_analytics;
            if !self.show_analytics {
                self.cancel_analytics_scan();
                self.cancel_reclaim_scan();
            }
            ui.close_menu();
        }
        if ui.button("Tastenkürzel   F1").clicked() {
            self.show_help = true;
            ui.close_menu();
        }
    }
}
