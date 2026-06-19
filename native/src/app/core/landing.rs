use super::prelude::*;
use super::*;

enum LandingAction {
    OpenLocation(String),
    Connect(crate::creds::SavedConnection),
    OpenGDrive,
    NewConnection,
    BuildIndex,
    RefreshIndex,
    ShowSyncJobs,
}

impl App {
    pub(in crate::app) fn show_landing_page(&mut self) {
        if let Some(h) = self.scan_handle.take() {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.scan_rx = None;
        self.scan_running = false;
        self.root_path.clear();
        self.entries = Vec::new();
        self.view = Vec::new();
        self.selection.clear();
        self.last_anchor = None;
        self.cursor = None;
        self.progress = empty_progress();
        self.failed_paths = Vec::new();
        self.summary_cache = None;
        self.sel_size_cache = (usize::MAX, usize::MAX, 0);
        self.view_dirty = false;
        self.band_press = None;
        self.band_active = false;
        self.remote = None;
        self.net_conn = None;
        self.path_edit_mode = false;
        self.text_draft.clear();
        self.ext_draft.clear();
        self.size_min_draft.clear();
        self.size_max_draft.clear();
        self.filter = FilterDef::new();
        self.filter_pending_at = None;
        self.mtime_min_date = None;
        self.mtime_max_date = None;
        self.btime_min_date = None;
        self.btime_max_date = None;
        self.folder_search_query.clear();
        self.folder_search_results.clear();
        self.folder_search_rx = None;
        self.folder_search_seq += 1;
        self.omni_sel = None;
        self.omni_activate = None;
    }

    pub(in crate::app) fn navigate_to_landing_page(&mut self) {
        if self.root_path.is_empty() && self.remote.is_none() && self.net_conn.is_none() {
            return;
        }
        if !self.root_path.is_empty() {
            self.history.push(self.root_path.clone());
            self.forward.clear();
            if self.history.len() > 100 {
                self.history.remove(0);
            }
        }
        self.show_landing_page();
    }

    pub(in crate::app) fn ui_current_content(&mut self, ui: &mut egui::Ui) {
        if self.root_path.is_empty() && !self.scan_running {
            self.ui_landing(ui);
        } else {
            self.ui_table(ui);
        }
    }

    pub(in crate::app) fn ui_landing(&mut self, ui: &mut egui::Ui) {
        let mut action: Option<LandingAction> = None;
        let common = self.landing_common_folders();
        let recent = self.recent.clone();
        let favorites = self.favorites.clone();
        let drives = self.drive_info.clone();
        let connections: Vec<crate::creds::SavedConnection> =
            self.saved_connections.iter().rev().cloned().collect();
        let gdrive_connected = crate::cloud::is_connected(crate::cloud::Provider::GDrive);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("Startseite");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.index_building {
                            ui.add(egui::Spinner::new().size(14.0));
                            ui.label(
                                RichText::new(format!("Index: {} Ordner", self.index_progress))
                                    .small()
                                    .color(Color32::from_gray(145)),
                            );
                        } else if self.folder_index.is_empty() {
                            if ui.small_button("Index bauen").clicked() {
                                action = Some(LandingAction::BuildIndex);
                            }
                        } else if ui.small_button("Index aktualisieren").clicked() {
                            action = Some(LandingAction::RefreshIndex);
                        }
                    });
                });
                ui.separator();
                ui.add_space(6.0);

                ui.horizontal_wrapped(|ui| {
                    if ui.button("Ordner waehlen").clicked() {
                        let init = self.root_path.clone();
                        self.open_picker(PickerPurpose::ScanFolder, &init);
                    }
                    if ui.button("Neue Verbindung").clicked() {
                        action = Some(LandingAction::NewConnection);
                    }
                    if ui.button("Sync-Jobs").clicked() {
                        self.show_sync_jobs = true;
                    }
                    if ui.button("Teilen").clicked() {
                        self.show_share = true;
                    }
                });
                ui.add_space(14.0);

                if ui.available_width() >= 760.0 {
                    ui.columns(2, |columns| {
                        self.ui_landing_left(
                            &mut columns[0],
                            &common,
                            &recent,
                            &favorites,
                            &drives,
                            &mut action,
                        );
                        self.ui_landing_right(
                            &mut columns[1],
                            &connections,
                            gdrive_connected,
                            &mut action,
                        );
                    });
                } else {
                    self.ui_landing_left(ui, &common, &recent, &favorites, &drives, &mut action);
                    ui.add_space(12.0);
                    self.ui_landing_right(ui, &connections, gdrive_connected, &mut action);
                }
            });

        if let Some(action) = action {
            match action {
                LandingAction::OpenLocation(path) => self.navigate_to_location(&path),
                LandingAction::Connect(c) => self.connect_saved(&c),
                LandingAction::OpenGDrive => self.open_gdrive_browse(),
                LandingAction::NewConnection => {
                    self.connect_form = crate::connect::ConnectForm::default();
                    self.show_connect = true;
                }
                LandingAction::BuildIndex | LandingAction::RefreshIndex => self.start_index_build(),
                LandingAction::ShowSyncJobs => self.show_sync_jobs = true,
            }
        }
    }

    fn ui_landing_left(
        &self,
        ui: &mut egui::Ui,
        common: &[(String, String)],
        recent: &[String],
        favorites: &[String],
        drives: &[(String, u64, u64)],
        action: &mut Option<LandingAction>,
    ) {
        ui_landing_section(ui, "Schnellzugriff", |ui| {
            for (label, path) in common {
                if landing_row(ui, label, path, false).clicked() {
                    *action = Some(LandingAction::OpenLocation(path.clone()));
                }
            }
        });

        ui.add_space(12.0);
        ui_landing_section(ui, "Zuletzt", |ui| {
            if recent.is_empty() {
                ui.colored_label(Color32::from_gray(125), "Noch keine Ordner");
            } else {
                for path in recent.iter().take(8) {
                    let label = landing_basename(path);
                    if landing_row(ui, &label, path, false).clicked() {
                        *action = Some(LandingAction::OpenLocation(path.clone()));
                    }
                }
            }
        });

        if !favorites.is_empty() {
            ui.add_space(12.0);
            ui_landing_section(ui, "Favoriten", |ui| {
                for path in favorites.iter().take(8) {
                    let label = landing_basename(path);
                    if landing_row(ui, &label, path, false).clicked() {
                        *action = Some(LandingAction::OpenLocation(path.clone()));
                    }
                }
            });
        }

        if !drives.is_empty() {
            ui.add_space(12.0);
            ui_landing_section(ui, "Laufwerke", |ui| {
                for (drive, free, total) in drives {
                    let detail = if *total > 0 {
                        format!("{} frei von {}", format_bytes(*free), format_bytes(*total))
                    } else {
                        String::new()
                    };
                    if landing_row(ui, drive, &detail, false).clicked() {
                        *action = Some(LandingAction::OpenLocation(drive.clone()));
                    }
                    if *total > 0 {
                        let used = total.saturating_sub(*free);
                        let frac = used as f32 / *total as f32;
                        ui.add(
                            egui::ProgressBar::new(frac)
                                .desired_width(ui.available_width())
                                .desired_height(5.0),
                        );
                    }
                }
            });
        }
    }

    fn ui_landing_right(
        &self,
        ui: &mut egui::Ui,
        connections: &[crate::creds::SavedConnection],
        gdrive_connected: bool,
        action: &mut Option<LandingAction>,
    ) {
        ui_landing_section(ui, "Verbindungen", |ui| {
            if gdrive_connected && landing_row(ui, "Google Drive", "gdrive://", false).clicked() {
                *action = Some(LandingAction::OpenGDrive);
            }
            if connections.is_empty() && !gdrive_connected {
                ui.colored_label(Color32::from_gray(125), "Noch keine Verbindungen");
            }
            for c in connections.iter().take(10) {
                let title = c.display();
                let detail = c.to_target();
                if landing_row(ui, &title, &detail, false).clicked() {
                    *action = Some(LandingAction::Connect(c.clone()));
                }
            }
        });

        if !self.sync_jobs.is_empty() {
            ui.add_space(12.0);
            ui_landing_section(ui, "Sync-Jobs", |ui| {
                for job in self.sync_jobs.iter().take(8) {
                    let detail = format!("{}  <->  {}", job.source, job.target);
                    if landing_row(ui, &job.name, &detail, false).clicked() {
                        *action = Some(LandingAction::ShowSyncJobs);
                    }
                }
                if ui.button("Sync-Jobs anzeigen").clicked() {
                    *action = Some(LandingAction::ShowSyncJobs);
                }
            });
        }
    }

    fn landing_common_folders(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        out.push((
            "Home".to_string(),
            self.home.to_string_lossy().replace('\\', "/"),
        ));
        for (label, sub) in [
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Downloads", "Downloads"),
            ("Pictures", "Pictures"),
            ("Music", "Music"),
            ("Videos", "Videos"),
        ] {
            let path = self.home.join(sub);
            if path.exists() {
                out.push((label.to_string(), path.to_string_lossy().replace('\\', "/")));
            }
        }
        out
    }
}

fn ui_landing_section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(
        RichText::new(title)
            .strong()
            .small()
            .color(Color32::from_gray(150)),
    );
    ui.add_space(3.0);
    add(ui);
}

fn landing_row(ui: &mut egui::Ui, title: &str, detail: &str, selected: bool) -> egui::Response {
    let width = ui.available_width().max(160.0);
    let height = if detail.is_empty() { 32.0 } else { 44.0 };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else if response.hovered() {
            visuals.bg_fill
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect.shrink(1.0), 4.0, fill);
        }
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            4.0,
            egui::Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color),
        );

        let title_rect = egui::Rect::from_min_max(
            rect.left_top() + egui::vec2(8.0, 5.0),
            egui::pos2(rect.right() - 8.0, rect.top() + 24.0),
        );
        paint_landing_text(
            ui,
            title_rect,
            title,
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );

        if !detail.is_empty() {
            let detail_rect = egui::Rect::from_min_max(
                rect.left_top() + egui::vec2(8.0, 24.0),
                rect.right_bottom() - egui::vec2(8.0, 4.0),
            );
            paint_landing_text(
                ui,
                detail_rect,
                detail,
                egui::TextStyle::Small.resolve(ui.style()),
                Color32::from_gray(135),
            );
        }
    }

    response.on_hover_text(detail)
}

fn paint_landing_text(
    ui: &egui::Ui,
    rect: egui::Rect,
    content: &str,
    font_id: egui::FontId,
    color: Color32,
) {
    if content.is_empty() {
        return;
    }
    use egui::text::{LayoutJob, TextWrapping};
    let mut job = LayoutJob::simple_singleline(content.to_string(), font_id, color);
    job.wrap = TextWrapping::truncate_at_width(rect.width().max(8.0));
    let galley = ui.fonts(|f| f.layout_job(job));
    ui.painter().galley(rect.left_top(), galley, color);
}

fn landing_basename(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if base.is_empty() {
        path.to_string()
    } else {
        base.to_string()
    }
}
