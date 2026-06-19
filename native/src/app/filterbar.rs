use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ui_filterbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("textmode")
                .selected_text(match self.filter.text_mode {
                    TextMode::Substring => "enthält",
                    TextMode::Regex => "RegExp",
                    TextMode::Glob => "Glob",
                })
                .show_ui(ui, |ui| {
                    let mut changed = false;
                    changed |= ui
                        .selectable_value(&mut self.filter.text_mode, TextMode::Substring, "enthält")
                        .clicked();
                    changed |= ui
                        .selectable_value(&mut self.filter.text_mode, TextMode::Regex, "RegExp")
                        .clicked();
                    changed |= ui
                        .selectable_value(&mut self.filter.text_mode, TextMode::Glob, "Glob")
                        .clicked();
                    if changed {
                        self.recompute_view();
                    }
                });

            // Server-side recursive search (SSH agent): only on a remote whose
            // backend supports it, with a non-regex query typed.
            let show_server_search =
                self.remote.as_ref().map_or(false, |rs| rs.backend.supports_search());
            if show_server_search {
                let q = self.text_draft.trim().to_string();
                let enabled = !q.is_empty() && self.filter.text_mode != TextMode::Regex;
                if ui
                    .add_enabled(enabled, egui::Button::new("🔎 Server"))
                    .on_hover_text(
                        "Rekursive Suche serverseitig über den Agent — durchsucht den ganzen \
                         Unterbaum und liefert nur die Treffer (enthält/Glob).",
                    )
                    .clicked()
                {
                    self.run_remote_search(q);
                }
            }

            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.text_draft)
                    .hint_text(match self.filter.text_mode {
                        TextMode::Substring => "Filtern · /Ordnersuche · Pfad · ›Befehl · .. hoch",
                        TextMode::Regex => "Regex z.B. \\.log$",
                        TextMode::Glob => "Glob z.B. **/build/**",
                    })
                    .desired_width(300.0),
            );
            let field_rect = resp.rect;
            if self.name_filter_focus || self.folder_search_focus {
                resp.request_focus();
                self.name_filter_focus = false;
                self.folder_search_focus = false;
            }
            if resp.changed() {
                self.filter_pending_at = Some(Instant::now());
                self.omni_sel = None;
                // Folder-search runs ONLY in `/`-mode, so plain filter typing
                // never pops the dropdown (and the arrows stay with the list).
                let q = if omni_mode(&self.text_draft) == OmniMode::FolderSearch {
                    self.text_draft.trim_start().trim_start_matches('/').trim().to_string()
                } else {
                    String::new()
                };
                if !q.is_empty() {
                    self.folder_search_query = q;
                    self.folder_search_pending_at = Some(std::time::Instant::now());
                } else {
                    self.folder_search_query.clear();
                    self.folder_search_results.clear();
                    self.folder_search_pending_at = None;
                }
            }
            // Enter drives navigation/commands (handled in `update` after the
            // frame's view + folder-search hits have settled).
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.filter_enter = true;
            }
            // Dropdown: roots, commands, and folder-search jumps.
            if resp.has_focus() {
                let items = self.build_omni_items();
                if !items.is_empty() {
                    let (down, up) = ui.input_mut(|i| {
                        (
                            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                        )
                    });
                    if down {
                        self.omni_sel = Some(match self.omni_sel {
                            Some(s) => (s + 1).min(items.len() - 1),
                            None => 0,
                        });
                    }
                    if up {
                        self.omni_sel = match self.omni_sel {
                            Some(0) | None => None,
                            Some(s) => Some(s - 1),
                        };
                    }
                    let sel = self.omni_sel;
                    let mut clicked: Option<OmniAction> = None;
                    egui::Area::new(egui::Id::new("omni_popup"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(field_rect.left_bottom() + egui::vec2(0.0, 3.0))
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.set_min_width(field_rect.width().max(680.0));
                                egui::ScrollArea::vertical()
                                    .id_salt("omni_results")
                                    .max_height(520.0)
                                    .show(ui, |ui| {
                                        for (i, it) in items.iter().enumerate() {
                                            let r = ui
                                                .selectable_label(
                                                    Some(i) == sel,
                                                    format!("{}  {}", it.icon, it.label),
                                                )
                                                .on_hover_text(&it.sub);
                                            if r.clicked() {
                                                clicked = Some(it.action.clone());
                                            }
                                        }
                                    });
                            });
                        });
                    if let Some(a) = clicked {
                        self.omni_activate = Some(a);
                    }
                }
            }

            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.ext_draft)
                    .hint_text("Endungen z.B. jpg,png")
                    .desired_width(180.0),
            );
            if resp.changed() {
                self.filter_pending_at = Some(Instant::now());
            }

            ui.label("Größe:");
            self.size_input(ui, "size_min", "≥ 10 MB", true);
            self.size_input(ui, "size_max", "≤ 1 GB", false);

            ui.label("Geändert:");
            self.date_filter_ui(ui, true);

            ui.label("Erstellt:");
            self.date_filter_ui(ui, false);

            // Quick presets for the modified-date range
            let mut preset: Option<(Option<chrono::NaiveDate>, Option<chrono::NaiveDate>)> = None;
            egui::ComboBox::from_id_salt("date_preset")
                .selected_text("⏱ Zeitraum")
                .width(110.0)
                .show_ui(ui, |ui| {
                    let today = chrono::Local::now().date_naive();
                    if ui.button("Heute").clicked() {
                        preset = Some((Some(today), None));
                    }
                    if ui.button("Letzte 7 Tage").clicked() {
                        preset = Some((Some(today - chrono::Duration::days(7)), None));
                    }
                    if ui.button("Letzte 30 Tage").clicked() {
                        preset = Some((Some(today - chrono::Duration::days(30)), None));
                    }
                    if ui.button("Dieses Jahr").clicked() {
                        preset = Some((
                            chrono::NaiveDate::from_ymd_opt(
                                chrono::Datelike::year(&today),
                                1,
                                1,
                            ),
                            None,
                        ));
                    }
                    if ui.button("Alle Daten löschen").clicked() {
                        preset = Some((None, None));
                    }
                });
            if let Some((min, max)) = preset {
                self.mtime_min_date = min;
                self.mtime_max_date = max;
                if min.is_none() && max.is_none() {
                    self.btime_min_date = None;
                    self.btime_max_date = None;
                }
                self.apply_date_filters();
                self.recompute_view();
            }
        });

        ui.horizontal(|ui| {
            let mut changed = false;
            changed |= ui.checkbox(&mut self.filter.include_files, "Dateien").changed();
            changed |= ui.checkbox(&mut self.filter.include_dirs, "Ordner").changed();
            changed |= ui.checkbox(&mut self.filter.include_hidden, "versteckt").changed();
            changed |= ui.checkbox(&mut self.filter.include_system, "System").changed();
            if changed {
                self.recompute_view();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Reset").clicked() {
                    self.filter = FilterDef::new();
                    self.text_draft.clear();
                    self.ext_draft.clear();
                    self.size_min_draft.clear();
                    self.size_max_draft.clear();
                    self.mtime_min_date = None;
                    self.mtime_max_date = None;
                    self.btime_min_date = None;
                    self.btime_max_date = None;
                    self.filter_pending_at = None;
                    self.recompute_view();
                }
                if self.filter_is_active() {
                    ui.colored_label(Color32::from_rgb(255, 190, 90), "● Filter aktiv");
                }
                ui.label(
                    RichText::new(format!(
                        "{} / {} Einträge",
                        self.view.len(),
                        self.entries.len()
                    ))
                    .color(Color32::from_gray(140)),
                );
            });
        });
    }

    pub(in crate::app) fn size_input(&mut self, ui: &mut egui::Ui, id: &str, hint: &str, is_min: bool) {
        let draft = if is_min {
            &mut self.size_min_draft
        } else {
            &mut self.size_max_draft
        };
        let resp = ui.add(
            egui::TextEdit::singleline(draft)
                .id(egui::Id::new(id))
                .hint_text(hint)
                .desired_width(90.0),
        );
        if resp.lost_focus() {
            let parsed = parse_size_input(draft);
            if is_min {
                self.filter.size.min = parsed;
            } else {
                self.filter.size.max = parsed;
            }
            self.recompute_view();
        }
    }

    /// Calendar-based date range input: a "von 📅"/"bis 📅" button that turns
    /// into a date-picker button + clear once set.
    pub(in crate::app) fn date_filter_ui(&mut self, ui: &mut egui::Ui, is_mtime: bool) {
        let mut changed = false;
        for is_min in [true, false] {
            let id = format!(
                "dp_{}_{}",
                if is_mtime { "m" } else { "b" },
                if is_min { "min" } else { "max" }
            );
            let field = match (is_mtime, is_min) {
                (true, true) => &mut self.mtime_min_date,
                (true, false) => &mut self.mtime_max_date,
                (false, true) => &mut self.btime_min_date,
                (false, false) => &mut self.btime_max_date,
            };
            match field {
                Some(d) => {
                    let resp = ui.add(
                        egui_extras::DatePickerButton::new(d)
                            .id_salt(id.as_str())
                            .show_icon(false),
                    );
                    if resp.changed() {
                        changed = true;
                    }
                    if ui.small_button("✕").clicked() {
                        *field = None;
                        changed = true;
                    }
                }
                None => {
                    let label = if is_min { "von 📅" } else { "bis 📅" };
                    if ui.small_button(label).clicked() {
                        *field = Some(chrono::Local::now().date_naive());
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.apply_date_filters();
            self.recompute_view();
        }
    }

    pub(in crate::app) fn apply_date_filters(&mut self) {
        self.filter.mtime.min = self.mtime_min_date.map(date_to_ms_start);
        self.filter.mtime.max = self.mtime_max_date.map(date_to_ms_end);
        self.filter.btime.min = self.btime_min_date.map(date_to_ms_start);
        self.filter.btime.max = self.btime_max_date.map(date_to_ms_end);
    }

}
