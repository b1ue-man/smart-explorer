use super::landing_tiles::{ui_landing_actions, ui_landing_section, LandingAction, LandingTile};
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
        self.scan_retention = None;
        self.scan_truncated = false;
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
        let place_tiles = self.landing_place_tiles(&common, &recent, &favorites, &[]);
        let drive_tiles = self.landing_place_tiles(&[], &[], &[], &drives);
        let remote_tiles = self.landing_remote_tiles(&connections, gdrive_connected);
        let sync_tiles = self.landing_sync_tiles(&sync_results);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.heading("Startseite");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Ordnerindex…").clicked() {
                            self.open_settings(settings_ui::SettingsPage::Storage);
                        }
                    });
                });
                ui.separator();
                ui_landing_actions(ui, &action_tiles, &mut action);
                ui.add_space(6.0);
                ui_landing_section(ui, "Orte", true, &place_tiles, &mut action);
                if !drives.is_empty() {
                    ui_landing_section(ui, "Laufwerke", true, &drive_tiles, &mut action);
                }
                if !connections.is_empty() || gdrive_connected {
                    ui_landing_section(ui, "Remotes", true, &remote_tiles, &mut action);
                }
                if !self.sync_jobs.is_empty() {
                    ui_landing_section(ui, "Sync-Jobs", true, &sync_tiles, &mut action);
                }
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
                LandingAction::ShowSyncJobs => self.show_sync_jobs = true,
                LandingAction::ShowShare => self.show_share = true,
            }
        }
    }

    fn landing_action_tiles(&self) -> Vec<LandingTile> {
        vec![
            LandingTile::action(
                "Ordner öffnen",
                "Lokalen Ordner auswählen",
                "",
                LandingAction::ChooseFolder,
            ),
            LandingTile::action(
                "Neue Verbindung",
                "SFTP, FTP oder WebDAV",
                "",
                LandingAction::NewConnection,
            ),
            LandingTile::action(
                "Sync-Jobs",
                "Jobs verwalten, starten und vergleichen",
                "",
                LandingAction::ShowSyncJobs,
            ),
            LandingTile::action(
                "Share-Server",
                "Geräte und Freigaben verwalten",
                "",
                LandingAction::ShowShare,
            ),
        ]
    }

    fn landing_place_tiles(
        &self,
        common: &[(String, String)],
        recent: &[String],
        favorites: &[String],
        drives: &[(String, u64, u64)],
    ) -> Vec<LandingTile> {
        let mut tiles = Vec::new();
        let mut seen = HashSet::new();
        for (paths, category) in [(favorites, "Favorit"), (recent, "Zuletzt geöffnet")] {
            for path in paths {
                if seen.insert(path.clone()) {
                    tiles.push(LandingTile::action(
                        self.location_label(path),
                        path,
                        category,
                        LandingAction::OpenLocation(path.clone()),
                    ));
                }
            }
        }
        for (label, path) in common {
            if seen.insert(path.clone()) {
                tiles.push(LandingTile::action(
                    label,
                    path,
                    "",
                    LandingAction::OpenLocation(path.clone()),
                ));
            }
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
            tiles.push(LandingTile::status(
                "Keine Orte",
                "Noch keine Ordner geöffnet",
            ));
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
        for c in connections {
            tiles.push(LandingTile::action(
                c.display(),
                c.to_target(),
                "Gespeichert",
                LandingAction::Connect(c.clone()),
            ));
        }
        if tiles.is_empty() {
            tiles.push(LandingTile::status(
                "Keine Verbindungen",
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
        for job in &self.sync_jobs {
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
            "Persönlicher Ordner".to_string(),
            self.home.to_string_lossy().replace('\\', "/"),
        ));
        for (label, sub) in [
            ("Desktop", "Desktop"),
            ("Dokumente", "Documents"),
            ("Downloads", "Downloads"),
            ("Bilder", "Pictures"),
            ("Musik", "Music"),
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
