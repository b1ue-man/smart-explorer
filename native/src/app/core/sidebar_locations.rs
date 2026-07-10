use super::prelude::*;
use super::*;

impl App {
    pub(super) fn ui_sidebar_locations(&mut self, ui: &mut egui::Ui) {
        ui.heading("Smart Explorer");
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

        // Folder search now lives in the combo-field at the top (Ctrl+F): type
        // to filter the list, with global folder jumps offered in its dropdown.
        ui.label(
            RichText::new("Ordnersuche → Suchleiste oben (Ctrl+F)")
                .small()
                .color(Color32::from_gray(140)),
        );

        ui.horizontal(|ui| {
            if self.index_building {
                ui.colored_label(
                    Color32::from_gray(140),
                    format!("⟳ Indizieren… {} Ordner", self.index_progress),
                );
                if ui.small_button("Stop").clicked() {
                    self.cancel_index_build();
                }
            } else if self.folder_index.is_empty() {
                ui.colored_label(Color32::from_gray(140), "Kein Index");
                if ui
                    .small_button("Bauen")
                    .on_hover_text("Scannt alle Laufwerke einmalig nach Ordnern (etwa 30-90s)")
                    .clicked()
                {
                    self.start_index_build();
                }
            } else {
                let count = self.folder_index.len();
                ui.colored_label(
                    Color32::from_gray(140),
                    format!(
                        "Index: {} Ordner",
                        count.to_string().chars().rev().enumerate().fold(
                            String::new(),
                            |acc, (i, c)| {
                                if i > 0 && i % 3 == 0 {
                                    format!("{}.{}", c, acc)
                                } else {
                                    format!("{}{}", c, acc)
                                }
                            }
                        )
                    ),
                );
                if ui
                    .small_button("⟳")
                    .on_hover_text("Index aktualisieren")
                    .clicked()
                {
                    self.start_index_build();
                }
            }
        });

        ui.add_space(8.0);

        // ─── Favorites (starred folders) ───────────────────────────────
        if !self.favorites.is_empty() {
            ui.label(
                RichText::new("★ FAVORITEN")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            let favs = self.favorites.clone();
            let mut nav: Option<String> = None;
            let mut unstar: Option<String> = None;
            for f in &favs {
                ui.horizontal(|ui| {
                    let label = {
                        let base = f.trim_end_matches('/').rsplit('/').next().unwrap_or(f);
                        if base.is_empty() {
                            f.as_str()
                        } else {
                            base
                        }
                    };
                    if ui
                        .selectable_label(self.location_key(&self.root_path) == *f, label)
                        .on_hover_text(f)
                        .clicked()
                    {
                        nav = Some(f.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("✕")
                            .on_hover_text("Aus Favoriten entfernen")
                            .clicked()
                        {
                            unstar = Some(f.clone());
                        }
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

        ui.label(
            RichText::new("SCHNELLZUGRIFF")
                .small()
                .color(Color32::from_gray(140)),
        );
        let home = self.home.clone();
        for (label, sub) in [
            ("Home", ""),
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Downloads", "Downloads"),
            ("Pictures", "Pictures"),
            ("Music", "Music"),
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

        if !self.drive_info.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new("LAUFWERKE")
                    .small()
                    .color(Color32::from_gray(140)),
            );
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
        }

        if !self.recent.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new("ZULETZT")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            let recent = self.recent.clone();
            for r in recent {
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
        }
    }
}
