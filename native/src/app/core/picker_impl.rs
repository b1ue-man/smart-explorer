use super::prelude::*;
use super::*;
use crate::app::theme;

#[path = "picker_navigation.rs"]
mod navigation;
use navigation::picker_child_path;

impl App {

    pub(in crate::app) fn ui_picker(&mut self, ctx: &egui::Context) {
        if self.picker.is_none() {
            return;
        }
        let mut open = true;
        let mut close = false;
        let mut choose = false;
        let mut enter: Option<String> = None;
        let mut go_up = false;
        let mut open_local: Option<String> = None;
        let mut open_conn: Option<crate::creds::SavedConnection> = None;
        let mut open_tab = None;
        let mut open_peer = None;
        let tab_locations = self.picker_tab_locations();
        let peer_locations = self.picker_peer_locations();

        let Some(st) = self.picker.as_ref() else {
            return;
        };
        let title = st.purpose.title();
        let geometry_key = st.purpose.geometry_key();
        let local_only = st.purpose.local_only();
        let home = self.home.to_string_lossy().replace('\\', "/");
        let drives = self.drive_info.clone();
        let conns = self.saved_connections.clone();
        let connecting = st.connecting;
        let listing = st.listing;
        let error = st.error.clone();
        let cwd = st.cwd.clone();
        let entries = st.entries.clone();
        let mut selected = st.selected.filter(|index| *index < entries.len());
        let conn_label = st.conn_label.clone();
        let value_preview = Self::picker_value(st);
        let has_loc = st.backend.is_some();
        let gdrive_connected = crate::cloud::is_connected(crate::cloud::Provider::GDrive);
        let mut open_gdrive = false;

        let initial_size = super::dialog_geometry::picker_initial_size(ctx);
        super::dialog_geometry::window(ctx, geometry_key, title)
            .id(egui::Id::new(title))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(initial_size)
            .vscroll(true)
            .max_size(theme::window_content_limit(ctx))
            .constrain_to(ctx.screen_rect().shrink(16.0))
            .show(ctx, |ui| {
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                {
                    close = true;
                }
                // Capture the parent's real height before entering a horizontal
                // layout: a horizontal row itself only offers one row of height.
                let footer = super::dialog_geometry::picker_footer_height(ui);
                let body_height =
                    (super::dialog_geometry::remaining_height(ui, 0.0) - footer).max(1.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), body_height),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        // ── Left: places ──
                        let places_width = (ui.available_width() * 0.3).min(220.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(places_width, body_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(places_width);
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                                egui::ScrollArea::vertical()
                                    .min_scrolled_height(0.0)
                                    .id_salt("picker_places")
                                    .max_height(body_height)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new("ORTE").small().color(theme::muted(ui)),
                                        );
                                        if ui.selectable_label(false, "🏠 Home").clicked() {
                                            open_local = Some(home.clone());
                                        }
                                        for (d, _f, _t) in &drives {
                                            if ui
                                                .selectable_label(false, format!("💽 {}", d))
                                                .clicked()
                                            {
                                                open_local = Some(d.clone());
                                            }
                                        }
                                        // Keep the same remote choices for every sync entry point.
                                        if !local_only {
                                            ui.add_space(6.0);
                                            ui.label(
                                                RichText::new("VERBINDUNGEN")
                                                    .small()
                                                    .color(theme::muted(ui)),
                                            );
                                            if conns.is_empty()
                                                && !gdrive_connected
                                                && peer_locations.is_empty()
                                                && tab_locations.is_empty()
                                            {
                                                ui.colored_label(theme::muted(ui), "(keine)");
                                            }
                                            if gdrive_connected
                                                && ui
                                                    .selectable_label(false, "☁ Google Drive")
                                                    .clicked()
                                            {
                                                open_gdrive = true;
                                            }
                                            for c in &conns {
                                                if ui
                                                    .selectable_label(
                                                        false,
                                                        format!("🖧 {}", c.display()),
                                                    )
                                                    .on_hover_text(c.to_target())
                                                    .clicked()
                                                {
                                                    open_conn = Some(c.clone());
                                                }
                                            }
                                            for (label, target) in &peer_locations {
                                                if ui
                                                    .selectable_label(
                                                        false,
                                                        format!("Share: {label}"),
                                                    )
                                                    .clicked()
                                                {
                                                    open_peer =
                                                        Some((label.clone(), target.clone()));
                                                }
                                            }
                                            for (index, location) in
                                                tab_locations.iter().enumerate()
                                            {
                                                if ui
                                                    .selectable_label(
                                                        false,
                                                        format!("Tab: {}", location.label),
                                                    )
                                                    .clicked()
                                                {
                                                    open_tab = Some(index);
                                                }
                                            }
                                        }
                                    });
                            },
                        );

                        ui.separator();

                        // ── Right: current folder ──
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), body_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_height(body_height);
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    if ui.button("⬆ Hoch").clicked() {
                                        go_up = true;
                                    }
                                    if !conn_label.is_empty() {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(format!("● {}", conn_label))
                                                    .color(theme::accent(ui)),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(&conn_label);
                                    }
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(if cwd.is_empty() {
                                            "—".to_string()
                                        } else {
                                            cwd.clone()
                                        })
                                        .monospace()
                                        .color(theme::muted(ui)),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&cwd);
                                ui.separator();
                                if connecting || listing {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(if connecting {
                                            "Verbinde…"
                                        } else {
                                            "Lade Ordner…"
                                        });
                                    });
                                } else if let Some(e) = &error {
                                    ui.add(
                                        egui::Label::new(RichText::new(e).color(theme::danger(ui)))
                                            .truncate(),
                                    )
                                    .on_hover_text(e);
                                } else if !has_loc {
                                    ui.colored_label(
                                        theme::muted(ui),
                                        "Links einen Ort oder eine Verbindung wählen.",
                                    );
                                } else {
                                    ui.label("");
                                }
                                let list = egui::ScrollArea::vertical()
                                    .min_scrolled_height(0.0)
                                    .id_salt("picker_list")
                                    .min_scrolled_height(0.0)
                                    .max_height(super::dialog_geometry::remaining_height(ui, 0.0))
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        for (index, name) in entries.iter().enumerate() {
                                            let response = ui.selectable_label(
                                                selected == Some(index),
                                                format!("📁 {}", name),
                                            );
                                            if response.clicked() {
                                                selected = Some(index);
                                            }
                                            let keyboard_open = response.has_focus()
                                                && ui.input_mut(|input| {
                                                    input.consume_key(
                                                        egui::Modifiers::NONE,
                                                        egui::Key::Enter,
                                                    )
                                                });
                                            if response.double_clicked() || keyboard_open {
                                                enter = Some(name.clone());
                                            }
                                        }
                                        if has_loc
                                            && entries.is_empty()
                                            && error.is_none()
                                            && !connecting
                                        {
                                            ui.colored_label(
                                                theme::muted(ui),
                                                "(keine Unterordner)",
                                            );
                                        }
                                    });
                                ctx.data_mut(|data| {
                                    data.insert_temp(
                                        egui::Id::new(("picker_list_height", geometry_key)),
                                        list.inner_rect.height(),
                                    )
                                });
                            },
                        );
                    },
                );

                ui.separator();
                let can_choose = has_loc && !connecting && !listing && !cwd.is_empty();
                ui.horizontal_wrapped(|ui| {
                    let can_choose = has_loc && !connecting && !listing && !cwd.is_empty();
                    if ui
                        .add_enabled(can_choose, egui::Button::new("✔ Diesen Ordner wählen"))
                        .clicked()
                    {
                        choose = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        close = true;
                    }
                });
                ui.add(
                    egui::Label::new(
                        RichText::new(if can_choose {
                            value_preview.as_str()
                        } else {
                            ""
                        })
                        .color(theme::muted(ui)),
                    )
                    .truncate(),
                )
                .on_hover_text(&value_preview);
            });

        if let Some(picker) = self.picker.as_mut() {
            picker.selected = selected;
        }

        // Apply deferred actions (outside the borrow of self.picker).
        if let Some(name) = enter {
            if let Some(p) = self.picker.as_mut() {
                p.cwd = picker_child_path(&p.cwd, &name);
            }
            self.picker_list();
        }
        if go_up {
            let parent = self
                .picker
                .as_ref()
                .and_then(|p| Self::picker_parent(&p.cwd));
            if let Some(par) = parent {
                if let Some(p) = self.picker.as_mut() {
                    p.cwd = par;
                }
                self.picker_list();
            }
        }
        if let Some(root) = open_local {
            self.picker_open_local(&root);
        }
        if let Some(c) = open_conn {
            self.picker_open_connection(&c);
        }
        if open_gdrive {
            self.picker_open_gdrive();
        }
        if let Some((label, target)) = open_peer {
            self.picker_open_peer(label, target);
        }
        if let Some(index) = open_tab {
            if let Some(location) = tab_locations.into_iter().nth(index) {
                self.picker_use_location(location);
            }
        }
        if choose {
            if let Some((cwd, remote)) = self
                .picker
                .as_ref()
                .map(|picker| (picker.cwd.clone(), picker.is_remote))
            {
                if !self.drive_consumer_available(
                    super::drive_snapshot::DriveConsumer::Picker,
                    &cwd,
                    remote,
                ) {
                    if let Some(picker) = self.picker.as_mut() {
                        picker.error = Some("Datenträger nicht verbunden".into());
                    }
                    return;
                }
            }
            if let Some(p) = self.picker.take() {
                let value = Self::picker_value(&p);
                let picked_account = p
                    .is_remote
                    .then(|| {
                        crate::connect::saved_location(&self.saved_connections, &value)
                            .map(|(connection, _)| connection.account())
                    })
                    .flatten();
                let native = value.replace('/', std::path::MAIN_SEPARATOR_STR);
                match p.purpose {
                    PickerPurpose::SyncSource => {
                        if let Some(ed) = self.job_editor.as_mut() {
                            ed.source = value;
                        }
                    }
                    PickerPurpose::SyncTarget => {
                        if let Some(ed) = self.job_editor.as_mut() {
                            ed.target = value;
                        }
                    }
                    PickerPurpose::ScanFolder => self.start_scan(PathBuf::from(native)),
                    PickerPurpose::AnalyticsFolder => {
                        // Reuse the picker's live backend for a remote target;
                        // otherwise analyse the local folder.
                        if p.is_remote {
                            if let Some(be) = p.backend.clone() {
                                self.start_analytics_source(StorageScanSource::remote_at(
                                    be,
                                    p.cwd.clone(),
                                    p.conn_label.clone(),
                                    (!p.endpoint_prefix.is_empty())
                                        .then(|| p.endpoint_prefix.clone()),
                                    picked_account.clone(),
                                ));
                            }
                        } else {
                            self.accept_drive_consumer(
                                super::drive_snapshot::DriveConsumer::Analytics,
                                &value,
                            );
                            self.start_analytics_scan(value);
                        }
                    }
                    PickerPurpose::ReclaimFolder => {
                        if p.is_remote {
                            if let Some(be) = p.backend.clone() {
                                self.start_reclaim_source(StorageScanSource::remote_at(
                                    be,
                                    p.cwd.clone(),
                                    p.conn_label.clone(),
                                    (!p.endpoint_prefix.is_empty())
                                        .then(|| p.endpoint_prefix.clone()),
                                    picked_account.clone(),
                                ));
                            }
                        } else {
                            self.accept_drive_consumer(
                                super::drive_snapshot::DriveConsumer::Reclaim,
                                &value,
                            );
                            self.start_reclaim_scan(value);
                        }
                    }
                    PickerPurpose::MirrorDest => {
                        if let Some(backend) = p.backend {
                            self.start_mirror(backend, p.cwd);
                        }
                    }
                    PickerPurpose::BisyncDest => {
                        if let Some(backend) = p.backend {
                            self.start_bisync(backend, p.cwd);
                        }
                    }
                    PickerPurpose::CopyDest => self.copy_dest = native,
                    PickerPurpose::DownloadTo { src, source } => {
                        self.download_to_picked(
                            src,
                            source,
                            p.backend,
                            p.is_remote,
                            p.cwd,
                            p.conn_label,
                        );
                    }
                }
            }
        } else if close || !open {
            self.picker = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::picker_child_path;

    #[test]
    fn picker_child_path_keeps_absolute_roots() {
        assert_eq!(picker_child_path("C:/", "Users"), "C:/Users");
        assert_eq!(picker_child_path("C:/Users", "Silas"), "C:/Users/Silas");
        assert_eq!(picker_child_path("/", "tmp"), "/tmp");
    }
}
