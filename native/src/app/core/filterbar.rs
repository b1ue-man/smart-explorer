use super::prelude::*;
use super::*;
use crate::app::theme;

impl App {
    pub(in crate::app) fn ui_filterbar(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = CompiledFilter::compile(&self.filter).error() {
            ui.colored_label(theme::warning(ui), error);
        }
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
                        .selectable_value(
                            &mut self.filter.text_mode,
                            TextMode::Substring,
                            "enthält",
                        )
                        .clicked();
                    changed |= ui
                        .selectable_value(&mut self.filter.text_mode, TextMode::Regex, "RegExp")
                        .clicked();
                    changed |= ui
                        .selectable_value(&mut self.filter.text_mode, TextMode::Glob, "Glob")
                        .clicked();
                    if changed {
                        self.filter_changed();
                    }
                });

            // Server-side recursive search (SSH agent): only on a remote whose
            // backend supports it, with a non-regex query typed.
            let show_server_search = self
                .remote
                .as_ref()
                .is_some_and(|rs| rs.backend.supports_search());
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

            let controls_width = if self.filter_is_active() || !self.text_draft.is_empty() { 250.0 } else { 120.0 };
            let search_width = (ui.available_size_before_wrap().x - controls_width).clamp(140.0, 560.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.text_draft)
                    .hint_text(match self.filter.text_mode {
                        TextMode::Substring => "Dateien suchen…",
                        TextMode::Regex => "Regex z.B. \\.log$",
                        TextMode::Glob => "Glob z.B. **/build/**",
                    })
                    .desired_width(search_width),
            );
            let resp = resp.on_hover_text("Name filtern · /Ordner suchen · Pfad öffnen · ›Befehl · .. eine Ebene hoch (Ctrl+F)");
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
                    self.text_draft
                        .trim_start()
                        .trim_start_matches('/')
                        .trim()
                        .to_string()
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
                                ui.set_max_width((ui.ctx().screen_rect().right() - field_rect.left() - 12.0).max(120.0));
                                ui.set_min_width(field_rect.width());
                                egui::ScrollArea::vertical()
                                    .id_salt("omni_results")
                                    .max_height((ui.ctx().screen_rect().bottom() - field_rect.bottom() - 24.0).clamp(80.0, 360.0))
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

            let active = self.filter_is_active();
            if (active || !self.text_draft.is_empty()) && ui.button("Zurücksetzen").clicked() {
                self.reset_filters();
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label("Dateityp:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.ext_draft)
                    .hint_text("z.B. jpg; *.blend; tar.gz")
                    .desired_width(180.0),
            );
            if resp.changed() {
                self.filter_pending_at = Some(Instant::now());
            }

            ui.label("Größe:");
            self.size_input(ui, "size_min", "≥ 10 MB", true);
            self.size_input(ui, "size_max", "≤ 1 GB", false);

            for (modified, label) in [(true, "Geändert:"), (false, "Erstellt:")] {
                ui.allocate_ui_with_layout(
                    egui::vec2(220.0, 22.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(label);
                        self.date_filter_ui(ui, modified);
                    },
                );
            }

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
                            chrono::NaiveDate::from_ymd_opt(chrono::Datelike::year(&today), 1, 1),
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
                self.filter_changed();
            }
        });

        ui.horizontal_wrapped(|ui| {
            let mut changed = false;
            changed |= ui
                .checkbox(&mut self.filter.include_files, "Dateien")
                .changed();
            changed |= ui
                .checkbox(&mut self.filter.include_dirs, "Ordner")
                .changed();
            changed |= ui
                .checkbox(&mut self.filter.include_hidden, "versteckt")
                .changed();
            changed |= ui
                .checkbox(&mut self.filter.include_system, "System")
                .changed();
            changed |= ui
                .checkbox(
                    &mut self.filter.problem_names_only,
                    "⚠ Nur problematische Namen",
                )
                .on_hover_text(
                    "Nur Einträge, deren Namen Windows nicht normal ansprechen kann: reservierte \
                     Gerätenamen (NUL, CON, AUX, PRN, COM1…, LPT1…, auch mit Endung), Namen mit \
                     Punkt oder Leerzeichen am Ende, ungültige Zeichen. Solche Einträge lassen \
                     sich hier löschen (Entf / Shift+Entf) und umbenennen (F2).",
                )
                .changed();
            if changed {
                self.filter_changed();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, hint) = if self.scan_retention.is_some() {
                    (
                        format!(
                            "{} Treffer · {} durchsucht",
                            self.tree.rows.len(),
                            self.progress.scanned
                        ),
                        "Rekursiv mit aktivem Filter: nicht passende Einträge wurden beim Scan \
                         verworfen und belegen weder Speicher noch das Scan-Limit. Ein Filter, \
                         der mehr zulässt, startet den Scan automatisch neu.",
                    )
                } else {
                    (
                        format!("{} / {} Einträge", self.tree.rows.len(), self.entries.len()),
                        "Sichtbare Einträge / geladene Einträge",
                    )
                };
                ui.label(RichText::new(text).color(theme::muted(ui)))
                    .on_hover_text(hint);
            });
        });
    }

    pub(in crate::app) fn reset_filters(&mut self) {
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
        self.folder_search_query.clear();
        self.folder_search_results.clear();
        self.folder_search_pending_at = None;
        self.folder_search_rx = None;
        self.folder_search_seq += 1;
        self.omni_sel = None;
        self.omni_activate = None;
        self.filter_changed();
    }

    pub(in crate::app) fn size_input(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        hint: &str,
        is_min: bool,
    ) {
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
            let slot = if is_min {
                &mut self.filter.size.min
            } else {
                &mut self.filter.size.max
            };
            if *slot != parsed {
                *slot = parsed;
                self.filter_changed();
            }
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
                    if ui.small_button("×").clicked() {
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
            self.filter_changed();
        }
    }

    pub(in crate::app) fn apply_date_filters(&mut self) {
        self.filter.mtime.min = self.mtime_min_date.map(date_to_ms_start);
        self.filter.mtime.max = self.mtime_max_date.map(date_to_ms_end);
        self.filter.btime.min = self.btime_min_date.map(date_to_ms_start);
        self.filter.btime.max = self.btime_max_date.map(date_to_ms_end);
    }
}
