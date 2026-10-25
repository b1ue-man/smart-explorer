use super::prelude::*;
use super::ui_preferences::ColorMode;
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SettingsPage {
    Appearance,
    Connections,
    Updates,
    Storage,
    Integration,
    #[default]
    ThisDevice,
    Devices,
    Rooms,
    Sync,
    Vaults,
    Drives,
    Background,
    Diagnostics,
}

#[derive(Default)]
pub(super) struct SettingsState {
    pub(super) open: bool,
    pub(super) page: SettingsPage,
    pub(super) ai_plugin: super::settings_ai_plugin::AiPluginPanel,
}

impl App {
    pub(super) fn open_settings(&mut self, page: SettingsPage) {
        self.settings.open = true;
        self.settings.page = page;
        if let Some(tab) = super::settings_navigation::default_share_tab(page) { self.share_tab = tab; }
    }

    pub(in crate::app) fn ui_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings.open;
        let bounds = ctx.screen_rect().shrink(16.0);
        super::dialog_geometry::window(ctx, "settings_ui.0", "Einstellungen")
            .id(egui::Id::new("Einstellungen"))
            .open(&mut open)
            .collapsible(false)
            .default_size([640.0, 400.0])
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(bounds)
            .show(ctx, |ui| {
                let before = self.settings.page;
                let selected_page = super::settings_navigation::canonical(before);
                let font = egui::TextStyle::Body.resolve(ui.style());
                if ui.available_width() < font.size * 44.0 {
                    let label = super::settings_navigation::AREAS.iter().find(|(page, _)| *page == selected_page).map(|(_, label)| *label).unwrap_or("Diagnose und Protokolle");
                    egui::ComboBox::from_id_salt("settings_area").selected_text(label).show_ui(ui, |ui| {
                        for (page, label) in super::settings_navigation::AREAS {
                            if ui.selectable_label(selected_page == page, label).clicked() { self.settings.page = page; }
                        }
                    });
                } else {
                    ui.horizontal_wrapped(|ui| {
                        for (page, label) in super::settings_navigation::AREAS {
                            if ui.selectable_label(selected_page == page, label).clicked() { self.settings.page = page; }
                        }
                    });
                }
                if before != self.settings.page {
                    if let Some(tab) = super::settings_navigation::default_share_tab(self.settings.page) { self.share_tab = tab; }
                }
                ui.separator();
                let pending = self.open_share_request_count();
                if pending > 0 {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(theme::warning(ui), format!("{pending} offene Zugriffsanfrage(n)"));
                        if ui.button("Anfragen beantworten …").clicked() {
                            self.settings.page = SettingsPage::Devices;
                            self.share_tab = 6;
                        }
                    });
                }
                egui::ScrollArea::vertical().min_scrolled_height(0.0)
                    .id_salt(("settings_page", self.settings.page as u8))
                    .auto_shrink([false, false])
                    .max_height(super::dialog_geometry::remaining_height(ui, 1.0))
                    .show(ui, |ui| match self.settings.page {
                        SettingsPage::Appearance => self.ui_settings_appearance(ui),
                        SettingsPage::Connections => {
                            self.ui_settings_share(ui);
                            if ui.button("Gespeicherte Verbindung hinzufügen …").clicked() {
                                self.show_connect = true;
                            }
                            for connection in self.saved_connections.clone() {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(&connection.label);
                                    if ui.button("Bearbeiten …").clicked() { self.open_connection_edit(&connection); }
                                });
                            }
                            self.ui_share_settings_content(ui);
                            egui::CollapsingHeader::new("Google Drive")
                                .id_salt("settings_cloud_v2")
                                .show(ui, |ui| self.ui_menu_cloud(ui));
                        }
                        SettingsPage::Updates => self.ui_settings_updates(ui),
                        SettingsPage::Storage => {
                            self.ui_settings_search(ui);
                            ui.separator();
                            self.ui_temp_recovery(ui);
                        }
                        SettingsPage::Integration => {
                            self.ui_settings_integration(ui);
                            self.ui_settings_ai_plugin(ui);
                            self.ui_desktop_settings(ui);
                        }
                        SettingsPage::ThisDevice | SettingsPage::Devices | SettingsPage::Rooms => {
                            self.ui_share_settings_content(ui);
                        }
                        SettingsPage::Sync => {
                            ui.label("Gespeicherte Sync-Setups, Zeitpläne und laufende Synchronisationen.");
                            if ui.button("Sync-Setups verwalten …").clicked() { self.show_sync_jobs = true; }
                            for job in &self.sync_jobs { ui.label(&job.name); }
                        }
                        SettingsPage::Vaults => self.ui_settings_vaults(ui),
                        SettingsPage::Drives => {
                            self.ui_settings_devices(ui);
                            if ui.button("Laufwerke verwalten …").clicked() { self.open_mount_manager(); }
                        }
                        SettingsPage::Background => self.ui_settings_background(ui),
                        SettingsPage::Diagnostics => {
                            self.ui_share_settings_content(ui);
                            if ui.button("Hintergrundprotokoll öffnen").clicked() { self.show_daemon_log = true; }
                            self.ui_settings_search(ui);
                            self.ui_temp_recovery(ui);
                        }
                    });
            });
        super::licenses_ui::show_licenses_window(ctx);
        self.settings.open = open;
    }

    fn ui_settings_vaults(&mut self, ui: &mut egui::Ui) {
        // The sidebar owns vault window driving; this page only opens its
        // existing actions, so the same windows are never drawn twice.
        ui.horizontal_wrapped(|ui| {
            if ui.button("Tresor erstellen …").clicked() { super::vault_create_ui::open_create(); }
            if ui.button("Vorhandenen Tresor hinzufügen …").clicked() { super::vault_create_ui::open_add(None); }
        });
        let rows = super::vault_feed::rows();
        if rows.is_empty() { ui.label("Die aktuelle Tresorübersicht enthält keine Einträge."); }
        for row in rows {
            ui.horizontal_wrapped(|ui| {
                ui.label(&row.label);
                ui.label(match row.state {
                    super::vault_models::RowState::Unlocked => "Entsperrt",
                    super::vault_models::RowState::Locked => "Gesperrt",
                    super::vault_models::RowState::Locking => "Wird gesperrt",
                    super::vault_models::RowState::Unavailable => "Nicht verfügbar",
                });
                if ui.button("Eigenschaften und Rechte …").clicked() { super::vault_settings_ui::open(&row.site); }
            });
            if let Some(reason) = row.reason { ui.colored_label(theme::warning(ui), reason); }
        }
    }

    fn ui_settings_appearance(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Farbschema");
        let previous = self.appearance;
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut self.appearance.mode, ColorMode::Light, "Hell");
            ui.radio_value(&mut self.appearance.mode, ColorMode::Dark, "Dunkel");
            ui.radio_value(
                &mut self.appearance.mode,
                ColorMode::System,
                "Wie im System",
            );
        });
        ui.label(
            RichText::new("Wird sofort auf alle Ansichten angewendet.").color(theme::muted(ui)),
        );
        theme::section(ui, "Dateiliste");
        ui.checkbox(&mut self.appearance.compact, "Kompakte Zeilen");
        ui.checkbox(
            &mut self.appearance.detailed_columns,
            "Pfad, Erstelldatum und Tiefe zusätzlich anzeigen",
        );
        ui.label(
            RichText::new("Bei einer Suche in Unterordnern erscheint der Pfad automatisch.")
                .color(theme::muted(ui)),
        );
        if self.appearance != previous {
            ui.ctx().set_theme(self.appearance.mode.preference());
            self.save_ui_state();
        }
    }

    fn ui_settings_search(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Ordnersuche");
        ui.label("Der Index findet Ordner über die Suchleiste. Mit / beginnend suchen.");
        ui.horizontal_wrapped(|ui| {
            if self.index_building {
                ui.spinner();
                ui.label(format!("{} Ordner erfasst", self.index_progress));
                if ui.button("Abbrechen").clicked() {
                    self.cancel_index_build();
                }
            } else {
                ui.label(format!("{} Ordner im Index", self.folder_index.len()));
                if ui
                    .button(if self.folder_index.is_empty() {
                        "Index erstellen"
                    } else {
                        "Index aktualisieren"
                    })
                    .clicked()
                {
                    self.start_index_build();
                }
            }
        });
    }
}
