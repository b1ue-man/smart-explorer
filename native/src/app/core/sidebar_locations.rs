use super::prelude::*;
use super::*;

impl App {
    pub(super) fn ui_sidebar_locations(&mut self, ui: &mut egui::Ui) {
        if location_row(ui, "Startseite", self.root_path.is_empty()
            && self.remote.is_none() && self.net_conn.is_none()).clicked() {
            self.navigate_to_landing_page();
        }

        if !self.favorites.is_empty() {
            theme::section(ui, "FAVORITEN");
            let mut navigate = None;
            let mut remove = None;
            for favorite in self.favorites.clone() {
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 24.0).max(1.0);
                    if sidebar_row(ui, &self.location_label(&favorite), width,
                        self.location_key(&self.root_path) == favorite)
                        .on_hover_text(&favorite).clicked() { navigate = Some(favorite.clone()); }
                    if ui.add(egui::Button::new("×").frame(false))
                        .on_hover_text("Aus Favoriten entfernen").clicked() { remove = Some(favorite); }
                });
            }
            if let Some(path) = navigate { self.navigate_to_location(&path); }
            if let Some(path) = remove { self.toggle_favorite(&path); }
        }

        theme::section(ui, "SCHNELLZUGRIFF");
        let home = self.home.clone();
        for (label, sub) in [
            ("Persönlicher Ordner", ""), ("Desktop", "Desktop"),
            ("Dokumente", "Documents"), ("Downloads", "Downloads"),
            ("Bilder", "Pictures"), ("Musik", "Music"), ("Videos", "Videos"),
        ] {
            let path = if sub.is_empty() { home.clone() } else { home.join(sub) };
            if location_row(ui, label, self.root_path == path.to_string_lossy().replace('\\', "/"))
                .on_hover_text(path.to_string_lossy()).clicked() { self.start_scan(path); }
        }

        if !self.drive_info.is_empty() {
            theme::section(ui, "LAUFWERKE");
            for (drive, free, total) in self.drive_info.clone() {
                let response = location_row(ui, &drive, self.root_path == drive.replace('\\', "/"));
                if response.clicked() { self.start_scan(PathBuf::from(&drive)); }
                if total > 0 {
                    response.on_hover_text(format!("{} frei von {}", format_bytes(free), format_bytes(total)));
                    ui.add(egui::ProgressBar::new(total.saturating_sub(free) as f32 / total as f32)
                        .desired_width(ui.available_width().min(150.0)).desired_height(4.0));
                }
            }
        }

        if !self.recent.is_empty() {
            ui.add_space(6.0);
            egui::CollapsingHeader::new(RichText::new("ZULETZT").small().strong().color(theme::muted(ui)))
                .id_salt("sidebar_recent_desktop").show(ui, |ui| {
                    for path in self.recent.clone().into_iter().take(5) {
                        let label = path.rsplit('/').find(|part| !part.is_empty()).unwrap_or(&path);
                        if location_row(ui, label, self.root_path == path).on_hover_text(&path).clicked() {
                            self.start_scan(PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR)));
                        }
                    }
                });
        }
    }
}

fn location_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    sidebar_row(ui, label, ui.available_width(), selected)
}

pub(super) fn sidebar_row(ui: &mut egui::Ui, label: &str, width: f32, selected: bool) -> egui::Response {
    ui.allocate_ui_with_layout(egui::vec2(width, 22.0),
        egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
            ui.visuals_mut().selection.stroke = egui::Stroke::NONE;
            let response = ui.add(egui::Button::new(label).selected(selected)
                .min_size(egui::vec2(width, 22.0)).truncate());
            if response.has_focus() {
                ui.painter().rect_stroke(response.rect.shrink(1.0), 0.0,
                    egui::Stroke::new(1.0, theme::accent(ui)));
            }
            response
        }).inner
}
