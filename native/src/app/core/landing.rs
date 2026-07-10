use super::landing_tiles::{ui_landing_section, LandingAction, LandingTile};
use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn show_landing_page(&mut self) {
        if let Some(h) = self.scan_handle.take() {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.scan_rx = None;
        self.scan_running = false;
        self.scan_was_canceled = false;
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
        let sync_results = crate::syncjobs::load_results();

        let action_tiles = self.landing_action_tiles();
        let place_tiles = self.landing_place_tiles(&common, &recent, &favorites, &drives);
        let remote_tiles = self.landing_remote_tiles(&connections, gdrive_connected);
        let sync_tiles = self.landing_sync_tiles(&sync_results);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("Startseite");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.ui_landing_index_chip(ui, &mut action);
                    });
                });
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(8.0);

                ui_landing_section(ui, "Aktionen", true, &action_tiles, &mut action);
                ui_landing_section(ui, "Orte", true, &place_tiles, &mut action);
                ui_landing_section(ui, "Remotes", true, &remote_tiles, &mut action);
                ui_landing_section(ui, "Sync-Jobs", true, &sync_tiles, &mut action);
            });

        if let Some(action) = action {
            match action {
                LandingAction::ChooseFolder => {
                    let init = self.root_path.clone();
                    self.open_picker(PickerPurpose::ScanFolder, &init);
                }
                LandingAction::OpenLocation(path) => self.navigate_to_location(&path),
                LandingAction::Connect(c) => self.connect_saved(&c),
                LandingAction::OpenGDrive => self.open_gdrive_browse(),
                LandingAction::NewConnection => {
                    self.connect_form = crate::connect::ConnectForm::default();
                    self.show_connect = true;
                }
                LandingAction::BuildIndex | LandingAction::RefreshIndex => self.start_index_build(),
                LandingAction::ShowSyncJobs => self.show_sync_jobs = true,
                LandingAction::ShowShare => self.show_share = true,
            }
        }
    }

    fn ui_landing_index_chip(&self, ui: &mut egui::Ui, action: &mut Option<LandingAction>) {
        if self.index_building {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(
                RichText::new(format!("Index: {} Ordner", self.index_progress))
                    .small()
                    .color(Color32::from_gray(145)),
            );
        } else if self.folder_index.is_empty() {
            if ui.small_button("Index bauen").clicked() {
                *action = Some(LandingAction::BuildIndex);
            }
        } else if ui.small_button("Index aktualisieren").clicked() {
            *action = Some(LandingAction::RefreshIndex);
        }
    }

    fn landing_action_tiles(&self) -> Vec<LandingTile> {
        let mut tiles = vec![
            LandingTile::action(
                "Ordner waehlen",
                "Lokalen Ordner oeffnen",
                "Browse",
                LandingAction::ChooseFolder,
            ),
            LandingTile::action(
                "Neue Verbindung",
                "SFTP, FTP, WebDAV oder Share",
                "Remote",
                LandingAction::NewConnection,
            ),
            LandingTile::action(
                "Sync-Jobs",
                "Jobs verwalten, starten, vergleichen",
                "Sync",
                LandingAction::ShowSyncJobs,
            ),
            LandingTile::action(
                "Teilen",
                "Peer-Share und Quick Share",
                "Share",
                LandingAction::ShowShare,
            ),
        ];
        if self.index_building {
            tiles.push(LandingTile::status(
                "Index laeuft",
                format!("{} Ordner erfasst", self.index_progress),
            ));
        } else if self.folder_index.is_empty() {
            tiles.push(LandingTile::action(
                "Index bauen",
                "Schnellere Ordnersuche vorbereiten",
                "Suche",
                LandingAction::BuildIndex,
            ));
        } else {
            tiles.push(LandingTile::action(
                "Index aktualisieren",
                format!("{} Ordner im Index", self.folder_index.len()),
                "Suche",
                LandingAction::RefreshIndex,
            ));
        }
        tiles
    }

    fn landing_place_tiles(
        &self,
        common: &[(String, String)],
        recent: &[String],
        favorites: &[String],
        drives: &[(String, u64, u64)],
    ) -> Vec<LandingTile> {
        let mut tiles = Vec::new();
        for (label, path) in common {
            tiles.push(LandingTile::action(
                label,
                path,
                "Schnellzugriff",
                LandingAction::OpenLocation(path.clone()),
            ));
        }
        for path in recent.iter().take(8) {
            let label = landing_basename(path);
            tiles.push(LandingTile::action(
                label,
                path,
                "Zuletzt",
                LandingAction::OpenLocation(path.clone()),
            ));
        }
        for path in favorites.iter().take(8) {
            let label = landing_basename(path);
            tiles.push(LandingTile::action(
                label,
                path,
                "Favorit",
                LandingAction::OpenLocation(path.clone()),
            ));
        }
        for (drive, free, total) in drives {
            let (detail, meter) = if *total > 0 {
                let used = total.saturating_sub(*free);
                (
                    format!("{} frei von {}", format_bytes(*free), format_bytes(*total)),
                    Some((
                        used as f32 / *total as f32,
                        format!("{} belegt", format_bytes(used)),
                    )),
                )
            } else {
                (String::new(), None)
            };
            let mut tile = LandingTile::action(
                drive,
                detail,
                "Laufwerk",
                LandingAction::OpenLocation(drive.clone()),
            );
            if let Some((fraction, label)) = meter {
                tile = tile.meter(fraction, label);
            }
            tiles.push(tile);
        }
        if tiles.is_empty() {
            tiles.push(LandingTile::status("Keine Orte", "Noch nichts geoeffnet"));
        }
        tiles
    }

    fn landing_remote_tiles(
        &self,
        connections: &[crate::creds::SavedConnection],
        gdrive_connected: bool,
    ) -> Vec<LandingTile> {
        let mut tiles = Vec::new();
        if gdrive_connected {
            tiles.push(LandingTile::action(
                "Google Drive",
                "gdrive://",
                "Cloud",
                LandingAction::OpenGDrive,
            ));
        }
        for c in connections.iter().take(10) {
            tiles.push(LandingTile::action(
                c.display(),
                c.to_target(),
                "Gespeichert",
                LandingAction::Connect(c.clone()),
            ));
        }
        if tiles.is_empty() {
            tiles.push(LandingTile::status(
                "Keine Remotes",
                "Neue Verbindung anlegen",
            ));
        }
        tiles
    }

    fn landing_sync_tiles(
        &self,
        results: &std::collections::BTreeMap<String, crate::syncjobs::JobResult>,
    ) -> Vec<LandingTile> {
        let mut tiles = Vec::new();
        for job in self.sync_jobs.iter().take(12) {
            let result = results.get(&job.id);
            let detail = format!("{}  <->  {}", job.source, job.target);
            let (meta, warn) = landing_sync_meta(job, result);
            tiles.push(
                LandingTile::action(job.name.clone(), detail, meta, LandingAction::ShowSyncJobs)
                    .warn(warn),
            );
        }
        if self.sync_jobs.is_empty() {
            tiles.push(LandingTile::status(
                "Keine Sync-Jobs",
                "Jobs koennen im Sync-Fenster angelegt werden",
            ));
        }
        tiles.push(LandingTile::action(
            "Sync-Jobs verwalten",
            "Editor, Vorschau, Konfliktmodus",
            "Oeffnen",
            LandingAction::ShowSyncJobs,
        ));
        tiles
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

fn landing_basename(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    if base.is_empty() {
        path.to_string()
    } else {
        base.to_string()
    }
}

fn landing_sync_meta(
    job: &crate::syncjobs::SyncJob,
    result: Option<&crate::syncjobs::JobResult>,
) -> (String, bool) {
    let last = result
        .map(|r| landing_time_secs(r.when))
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if job.last_run > 0 {
                Some(landing_time_secs(job.last_run))
            } else {
                None
            }
        })
        .unwrap_or_else(|| "noch nie".to_string());
    let mut warn = false;
    let status = match result {
        Some(r) => {
            warn = r.conflicts > 0 || r.errors > 0;
            format!(
                "{} | Konflikte: {} | Fehler: {} | {} -> {}",
                r.note, r.conflicts, r.errors, r.a_to_b, r.b_to_a
            )
        }
        None => "kein Ergebnis gespeichert".to_string(),
    };
    let enabled = if job.enabled { "aktiv" } else { "pausiert" };
    (format!("Zuletzt: {last} | {enabled} | {status}"), warn)
}

fn landing_time_secs(secs: i64) -> String {
    if secs <= 0 {
        String::new()
    } else {
        format_date(secs.saturating_mul(1000))
    }
}
