use crate::app::theme;
use super::prelude::*;
use super::*;

impl App {
    pub(super) fn ui_sidebar_locations(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Smart Explorer").strong().size(16.0));
        ui.add_space(4.0);
        if ui
            .selectable_label(
                self.root_path.is_empty() && self.remote.is_none() && self.net_conn.is_none(),
                "Startseite",
            )
            .clicked()
        {
            self.navigate_to_landing_page();
        }
        ui.add_space(6.0);

        // ─── Favorites (starred folders) ───────────────────────────────
        if !self.favorites.is_empty() {
            ui.label(
                RichText::new("Favoriten")
                    .small()
                    .color(theme::muted(ui)),
            );
            let favs = self.favorites.clone();
            let mut nav: Option<String> = None;
            let mut unstar: Option<String> = None;
            for f in &favs {
                ui.horizontal(|ui| {
                    let label = self.location_label(f);
                    if ui
                        .add_sized([(ui.available_width() - 42.0).max(40.0), 30.0], egui::Button::new(label).frame(false).selected(self.location_key(&self.root_path) == *f).truncate())
                        .on_hover_text(f)
                        .clicked()
                    {
                        nav = Some(f.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("⋯", |ui| {
                            if ui.button("Aus Favoriten entfernen").clicked() {
                                unstar = Some(f.clone()); ui.close_menu();
                            }
                        });
                    });
                });
            }
            if let Some(p) = nav {
                self.navigate_to_location(&p);
            }
            if let Some(p) = unstar {
                self.toggle_favorite(&p);
            }
            ui.add_space(8.0);
        }

        egui::CollapsingHeader::new("Schnellzugriff")
            .id_salt("sidebar_places_v2").show(ui, |ui| {
        let home = self.home.clone();
        for (label, sub) in [
            ("Persönlicher Ordner", ""),
            ("Desktop", "Desktop"),
            ("Dokumente", "Documents"),
            ("Downloads", "Downloads"),
            ("Bilder", "Pictures"),
            ("Musik", "Music"),
            ("Videos", "Videos"),
        ] {
            let p = if sub.is_empty() {
                home.clone()
            } else {
                home.join(sub)
            };
            if ui
                .selectable_label(
                    self.root_path == p.to_string_lossy().replace('\\', "/"),
                    label,
                )
                .on_hover_text(p.to_string_lossy())
                .clicked()
            {
                self.start_scan(p);
            }
        }

        });

        if !self.drive_info.is_empty() {
            egui::CollapsingHeader::new("Laufwerke")
                .id_salt("sidebar_drives_v2").show(ui, |ui| {
            let infos = self.drive_info.clone();
            for (d, free, total) in infos {
                if ui
                    .selectable_label(self.root_path == d.replace('\\', "/"), &d)
                    .clicked()
                {
                    self.start_scan(PathBuf::from(&d));
                }
                if total > 0 {
                    let used = total.saturating_sub(free);
                    let frac = used as f32 / total as f32;
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(150.0)
                            .desired_height(6.0),
                    )
                    .on_hover_text(format!(
                        "{} frei von {}",
                        format_bytes(free),
                        format_bytes(total)
                    ));
                }
            }
            });
        }

        egui::CollapsingHeader::new("Zuletzt geöffnet")
            .id_salt("sidebar_recent_v2").show(ui, |ui| {
        if !self.recent.is_empty() {
            let recent = self.recent.clone();
            for r in recent.into_iter().take(5) {
                let label = r.rsplit('/').next().unwrap_or(&r).to_string();
                let label = if label.is_empty() { r.clone() } else { label };
                if ui
                    .selectable_label(self.root_path == r, &label)
                    .on_hover_text(&r)
                    .clicked()
                {
                    self.start_scan(PathBuf::from(r.replace('/', std::path::MAIN_SEPARATOR_STR)));
                }
            }
        }            });

    }
}
