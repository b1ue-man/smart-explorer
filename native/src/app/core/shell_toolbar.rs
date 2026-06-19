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
        painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 110));
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

    // ─── Context menus ──────────────────────────────────────────────────

    #[cfg(windows)]
    pub(in crate::app) fn show_shell_menu_for(&mut self, clicked_path: &str, ctx: &egui::Context) {
        use crate::shell_menu::{MenuResult, OwnMenuItem};

        let clicked_arc: Arc<str> = Arc::from(clicked_path);
        let paths: Vec<String> =
            if self.selection.contains(&clicked_arc) && self.selection.len() > 1 {
                self.selection
                    .iter()
                    .map(|k| sel_key_path(k).replace('/', "\\"))
                    .collect()
            } else {
                vec![clicked_path.replace('/', "\\")]
            };

        let filter_active = self.filter_is_active();
        let own = vec![
            OwnMenuItem {
                id: menu_ids::COPY,
                label: if filter_active {
                    "Kopieren (mit Filter)".to_string()
                } else {
                    "Kopieren".to_string()
                },
            },
            OwnMenuItem {
                id: menu_ids::CUT,
                label: "Ausschneiden".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::COPY_PATH,
                label: "Pfad kopieren".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::COPY_TO,
                label: "Kopieren nach… (Filter + Struktur)".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::MOVE_TO,
                label: "Verschieben nach…".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::RENAME,
                label: "Umbenennen (F2)".to_string(),
            },
        ];

        // Offer a favorite toggle when the clicked entry is a folder.
        let clicked_fwd = clicked_path.replace('\\', "/");
        let clicked_is_dir = self
            .entries
            .iter()
            .any(|e| e.is_dir && e.path.as_ref() == clicked_fwd);
        let mut own = own;
        if clicked_is_dir {
            own.push(OwnMenuItem {
                id: menu_ids::TOGGLE_FAV,
                label: if self.is_favorite(&clicked_fwd) {
                    "☆ Aus Favoriten entfernen".to_string()
                } else {
                    "★ Zu Favoriten".to_string()
                },
            });
        } else if is_zip_name(&clicked_fwd) {
            own.push(OwnMenuItem {
                id: menu_ids::EXTRACT_ZIP,
                label: "📦 Hier entpacken".to_string(),
            });
        }

        match crate::shell_menu::show_for_paths(&paths, None, None, &own) {
            Ok(MenuResult::Own(id)) => match id {
                menu_ids::COPY => self.clipboard_copy_files(false),
                menu_ids::CUT => self.clipboard_copy_files(true),
                menu_ids::COPY_PATH => self.copy_paths_to_clipboard(ctx),
                menu_ids::COPY_TO => {
                    self.copy_mode_pending = CopyMode::Copy;
                    self.copy_open = true;
                }
                menu_ids::MOVE_TO => {
                    self.copy_mode_pending = CopyMode::Move;
                    self.copy_open = true;
                }
                menu_ids::RENAME => self.open_rename(),
                menu_ids::TOGGLE_FAV => self.toggle_favorite(&clicked_fwd),
                menu_ids::EXTRACT_ZIP => self.start_zip_extract(clicked_fwd.clone()),
                _ => {}
            },
            Ok(MenuResult::Shell) => {
                // The shell verb may have changed the directory (delete,
                // rename, …) — refresh.
                self.rescan();
            }
            _ => {}
        }
    }

    #[cfg(not(windows))]
    pub(in crate::app) fn show_shell_menu_for(&mut self, clicked_path: &str, _ctx: &egui::Context) {
        self.open_in_explorer(clicked_path);
    }

    #[cfg(windows)]
    pub(in crate::app) fn show_background_menu(&mut self) {
        use crate::shell_menu::{MenuResult, OwnMenuItem};
        if self.root_path.is_empty() {
            return;
        }
        let own = vec![
            OwnMenuItem {
                id: menu_ids::PASTE,
                label: "Einfügen".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::NEW_FOLDER,
                label: "Neuer Ordner (Ctrl+Shift+N)".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::SELECT_ALL,
                label: "Alles auswählen (Ctrl+A)".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::REFRESH,
                label: "Aktualisieren (F5)".to_string(),
            },
        ];
        let folder = self.root_path.replace('/', "\\");
        match crate::shell_menu::show_background_menu(&folder, &own) {
            Ok(MenuResult::Own(id)) => match id {
                menu_ids::PASTE => self.clipboard_paste_files(),
                menu_ids::NEW_FOLDER => self.create_new_folder(),
                menu_ids::SELECT_ALL => self.select_all(),
                menu_ids::REFRESH => self.rescan(),
                _ => {}
            },
            Ok(MenuResult::Shell) => self.rescan(),
            _ => {}
        }
    }

    #[cfg(not(windows))]
    pub(in crate::app) fn show_background_menu(&mut self) {}

    // ── UI ────────────────────────────────────────────────────────────────

    pub(in crate::app) fn ui_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let r = ui
                .add_enabled(!self.history.is_empty(), egui::Button::new("◀"))
                .on_hover_text("Zurück (Alt+←)");
            self.accel_push('B', r.rect, AccelAct::Back);
            if r.clicked() {
                self.navigate_back();
            }
            let r = ui
                .add_enabled(!self.forward.is_empty(), egui::Button::new("▶"))
                .on_hover_text("Vor (Alt+→)");
            self.accel_push('N', r.rect, AccelAct::Forward);
            if r.clicked() {
                self.navigate_forward();
            }
            let r = ui
                .add_enabled(!self.root_path.is_empty(), egui::Button::new("↑"))
                .on_hover_text("Eine Ebene hoch (Alt+↑ / Backspace)");
            self.accel_push('U', r.rect, AccelAct::Up);
            if r.clicked() {
                self.navigate_up();
            }

            let r = ui.button("📂").on_hover_text("Ordner auswählen");
            self.accel_push('O', r.rect, AccelAct::PickFolder);
            if r.clicked() {
                let init = self.root_path.clone();
                self.open_picker(PickerPurpose::ScanFolder, &init);
            }

            // ─── Breadcrumbs / editable path ───────────────────────────
            let crumb_w = (ui.available_width() - 660.0).max(160.0);
            if self.path_edit_mode {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.root_path)
                        .desired_width(crumb_w)
                        .hint_text("Pfad eingeben…"),
                );
                if self.path_edit_focus {
                    resp.request_focus();
                    self.path_edit_focus = false;
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter && !self.root_path.is_empty() {
                    self.path_edit_mode = false;
                    let p =
                        PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
                    self.start_scan(p);
                } else if resp.lost_focus() {
                    self.path_edit_mode = false;
                }
            } else {
                let mut nav_to: Option<String> = None;
                ui.allocate_ui(egui::vec2(crumb_w, 22.0), |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt("crumbs")
                        .max_width(crumb_w)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let prefix = self.root_prefix();
                                if prefix.is_empty() {
                                    ui.colored_label(
                                        Color32::from_gray(120),
                                        "Ordner wählen oder Pfad eingeben (Ctrl+L)",
                                    );
                                } else {
                                    // Keep the leading separator(s) so absolute
                                    // remote paths ("/home/…") and UNC ("//srv/…")
                                    // stay absolute when a crumb is clicked —
                                    // otherwise the root became relative and
                                    // failed with "Wurzel kann nicht gelesen werden".
                                    let lead: String =
                                        prefix.chars().take_while(|&c| c == '/').collect();
                                    let mut acc = lead;
                                    let segs: Vec<&str> =
                                        prefix.split('/').filter(|s| !s.is_empty()).collect();
                                    for (i, seg) in segs.iter().enumerate() {
                                        if i > 0 {
                                            ui.label(
                                                RichText::new("›").color(Color32::from_gray(110)),
                                            );
                                        }
                                        acc.push_str(seg);
                                        acc.push('/');
                                        let full = acc.clone();
                                        if ui.small_button(*seg).clicked() {
                                            nav_to = Some(full);
                                        }
                                    }
                                }
                            });
                        });
                });
                if ui
                    .small_button("✏")
                    .on_hover_text("Pfad bearbeiten (Ctrl+L)")
                    .clicked()
                {
                    self.path_edit_mode = true;
                    self.path_edit_focus = true;
                }
                if let Some(p) = nav_to {
                    self.start_scan(PathBuf::from(
                        p.trim_end_matches('/')
                            .replace('/', std::path::MAIN_SEPARATOR_STR),
                    ));
                }
            }

            if self.scan_running {
                if ui.button("⏹ Stop").clicked() {
                    self.cancel_scan();
                }
            } else if ui.button("⟳").on_hover_text("Aktualisieren (F5)").clicked() {
                self.rescan();
            }

            let was_recursive = self.recursive;
            ui.toggle_value(&mut self.recursive, "🔁 Rekursiv")
                .on_hover_text("Inkl. Unterordner durchsuchen (Ctrl+R)");
            if was_recursive != self.recursive && !self.root_path.is_empty() {
                self.rescan();
            }

            ui.separator();

            let has_sel = !self.selection.is_empty();
            // Grouped feature menus (moved off the sidebar). Copy/cut/paste stay
            // on Ctrl+C/X/V and the right-click menu — out of the nav bar.
            ui.menu_button("🔌 Verbindung", |ui| {
                ui.set_min_width(330.0);
                self.ui_menu_connect(ui);
            });
            ui.menu_button("⇄ Sync", |ui| {
                ui.set_min_width(330.0);
                self.ui_menu_sync(ui);
            });
            ui.menu_button("⚙ Einstellungen", |ui| {
                ui.set_min_width(350.0);
                self.ui_menu_settings(ui);
            });
            if ui
                .selectable_label(self.show_share, "📡 Teilen")
                .on_hover_text(
                    "Dateien direkt an gekoppelte Geräte / in Räume senden (P2P, verschlüsselt)",
                )
                .clicked()
            {
                self.show_share = !self.show_share;
            }
            ui.separator();
            if ui
                .add_enabled(has_sel, egui::Button::new("🗑").small())
                .on_hover_text("Entf — in Papierkorb")
                .clicked()
            {
                self.trash_selected();
            }
            // "Neu" dropdown: folder + various editable file types.
            enum NewKind {
                Folder,
                File(&'static str, &'static str),
            }
            let mut new_kind: Option<NewKind> = None;
            ui.add_enabled_ui(!self.root_path.is_empty(), |ui| {
                ui.menu_button("➕ Neu", |ui| {
                    if ui.button("📁 Ordner").clicked() {
                        new_kind = Some(NewKind::Folder);
                        ui.close_menu();
                    }
                    ui.separator();
                    for (label, base, ext) in [
                        ("📄 Textdatei (.txt)", "Neue Textdatei", "txt"),
                        ("📝 Markdown (.md)", "Neue Notiz", "md"),
                        ("📊 CSV (.csv)", "Neue Tabelle", "csv"),
                        ("🔧 JSON (.json)", "Neue Datei", "json"),
                        ("🌐 HTML (.html)", "Neue Seite", "html"),
                        ("</> Code (.rs)", "Neue Datei", "rs"),
                    ] {
                        if ui.button(label).clicked() {
                            new_kind = Some(NewKind::File(base, ext));
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Neu: Ordner oder Datei (Ctrl+Shift+N = Ordner)");
            });
            match new_kind {
                Some(NewKind::Folder) => self.create_new_folder(),
                Some(NewKind::File(base, ext)) => self.create_new_file(base, ext),
                None => {}
            }
            // Star the current folder
            let starred =
                !self.root_path.is_empty() && self.is_favorite(&self.location_key(&self.root_path));
            let star_glyph = if starred { "★" } else { "☆" };
            if ui
                .add_enabled(
                    !self.root_path.is_empty(),
                    egui::Button::new(star_glyph).small(),
                )
                .on_hover_text("Aktuellen Ordner zu Favoriten (Ctrl+B)")
                .clicked()
            {
                self.star_current_folder();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.toggle_value(&mut self.show_summary, "Σ").changed() {
                    self.save_ui_state();
                }
                if ui
                    .toggle_value(&mut self.dirs_first, "📁↑")
                    .on_hover_text(
                        "Ordner zuerst sortieren — gilt nur für DIESEN Ordner und wird dafür \
                         gemerkt. Aus: Dateien und Ordner gemischt nach der aktiven Spalte \
                         (z.B. nach Datum).",
                    )
                    .changed()
                {
                    // Bind the choice to the current location (connection+path).
                    if !self.root_path.is_empty() {
                        let key = self.location_key(&self.root_path);
                        self.dir_sort.insert(key, self.dirs_first);
                        save_dir_sort(&self.dir_sort);
                    }
                    self.recompute_view();
                }
                ui.toggle_value(&mut self.show_analytics, "📊")
                    .on_hover_text("Speicher-Analyse (Treemap, größte Ordner/Dateien)");
                if ui
                    .toggle_value(&mut self.show_filters, "🔍 Filter")
                    .on_hover_text("Filterleiste ein-/ausklappen")
                    .changed()
                {
                    self.save_ui_state();
                }
                if ui.button("？").on_hover_text("Tastenkürzel (F1)").clicked() {
                    self.show_help = !self.show_help;
                }
                let r = ui
                    .selectable_label(self.split, "⊟ Split")
                    .on_hover_text("Zwei Tabs nebeneinander (F6)");
                self.accel_push('S', r.rect, AccelAct::Split);
                if r.clicked() {
                    self.toggle_split();
                }
            });
        });
    }
}
