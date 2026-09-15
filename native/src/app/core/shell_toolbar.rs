use super::prelude::*;
use super::*;

impl App {
    /// Full-window hint shown while files are dragged over the app.
    pub(in crate::app) fn ui_drop_overlay(&self, ctx: &egui::Context) {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_overlay"),
        ));
        let rect = ctx.screen_rect();
        painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 200));
        let (text, color) = match self.drop_target() {
            Some(p) => (
                format!("📥 Hier ablegen → {}\n(Umschalt = verschieben)", p),
                Color32::from_rgb(150, 220, 255),
            ),
            None => (
                "Ablegen nur in einem lokalen Ordner möglich".to_string(),
                Color32::from_rgb(255, 185, 120),
            ),
        };
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(22.0),
            color,
        );
    }

    pub(in crate::app) fn ui_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (label, enabled, key, action, tip) in [
                ("←", !self.history.is_empty(), 'B', AccelAct::Back, "Zurück (Alt+←)"),
                ("→", !self.forward.is_empty(), 'N', AccelAct::Forward, "Vor (Alt+→)"),
                ("↑", !self.root_path.is_empty(), 'U', AccelAct::Up, "Eine Ebene hoch (Alt+↑)"),
            ] {
                let response = ui.add_enabled(enabled, egui::Button::new(label))
                    .on_hover_text(tip);
                self.accel_push(key, response.rect, action);
                if response.clicked() {
                    match action {
                        AccelAct::Back => self.navigate_back(),
                        AccelAct::Forward => self.navigate_forward(),
                        _ => self.navigate_up(),
                    }
                }
            }
            let pick = ui.button("Ordner…").on_hover_text("Ordner auswählen");
            self.accel_push('O', pick.rect, AccelAct::PickFolder);
            if pick.clicked() {
                let initial = self.root_path.clone();
                self.open_picker(PickerPurpose::ScanFolder, &initial);
            }

            let path_width = (ui.available_width() - 150.0).max(80.0);
            if self.path_edit_mode {
                let response = ui.add_sized([path_width, 30.0],
                    egui::TextEdit::singleline(&mut self.root_path).hint_text("Pfad eingeben…"));
                if self.path_edit_focus {
                    response.request_focus();
                    self.path_edit_focus = false;
                }
                if response.lost_focus() {
                    self.path_edit_mode = false;
                    if ui.input(|input| input.key_pressed(egui::Key::Enter)) && !self.root_path.is_empty() {
                        self.start_scan(PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR)));
                    }
                }
            } else {
                let mut destination = None;
                let colors = theme::palette(ui);
                egui::Frame::none().fill(colors.surface)
                    .stroke(egui::Stroke::new(1.0_f32, colors.control_border))
                    .rounding(6.0).inner_margin(egui::Margin::symmetric(8.0, 0.0))
                    .show(ui, |ui| {
                        ui.set_width((path_width - 18.0).max(40.0));
                        egui::ScrollArea::horizontal().id_salt("crumbs")
                            .max_width((path_width - 18.0).max(40.0)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if self.root_path.is_empty() {
                                        ui.add_sized([ui.available_width(), 30.0],
                                            egui::Label::new(RichText::new("Startseite").color(colors.muted)));
                                    } else {
                                        for (index, crumb) in navigation_path::breadcrumbs(&self.root_path).iter().enumerate() {
                                            if index > 0 { ui.label(RichText::new("›").color(colors.muted)); }
                                            if ui.add(egui::Button::new(&crumb.label).frame(false))
                                                .on_hover_text(&crumb.path).clicked() {
                                                destination = Some(crumb.path.clone());
                                            }
                                        }
                                    }
                                });
                            });
                    });
                if let Some(path) = destination {
                    self.start_scan(PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR)));
                }
            }
            if ui.button("Pfad").on_hover_text("Pfad bearbeiten (Ctrl+L)").clicked() {
                self.path_edit_mode = true;
                self.path_edit_focus = true;
            }
            if self.scan_running {
                if ui.button("■").on_hover_text("Scan abbrechen").clicked() { self.cancel_scan(); }
            } else if ui.button("⟳").on_hover_text("Aktualisieren (F5)").clicked() {
                self.rescan();
            }
            let starred = !self.root_path.is_empty() && self.is_favorite(&self.location_key(&self.root_path));
            if ui.add_enabled(!self.root_path.is_empty(), egui::Button::new(if starred { "★" } else { "☆" }))
                .on_hover_text("Ordner als Favorit speichern (Ctrl+B)").clicked() {
                self.star_current_folder();
            }
        });
    }
}
