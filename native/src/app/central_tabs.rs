use super::prelude::*;
use super::*;

impl App {
    /// Render the central area: a single table, or two side-by-side panes in
    /// split mode. Each pane renders via `ui_table`; the non-focused pane's
    /// tab state is swapped into the working fields just for its render.
    pub(in crate::app) fn ui_central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.split || self.tabs.len() < 2 {
                self.split = self.split && self.tabs.len() >= 2;
                self.pane_rects.clear();
                self.current_render_tab = self.active_tab;
                self.ui_table(ui);
                return;
            }
            let n = self.tabs.len();
            self.focused_pane = self.focused_pane.min(1);
            // Keep pane indices valid and ensure the focused pane shows the
            // active tab.
            for p in self.panes.iter_mut() {
                if *p >= n {
                    *p = 0;
                }
            }
            if self.panes[0] != self.active_tab && self.panes[1] != self.active_tab {
                self.panes[self.focused_pane] = self.active_tab;
            }
            if self.panes[0] == self.panes[1] {
                // Keep the focused pane's tab; move the other to a free one.
                let other = 1 - self.focused_pane;
                self.panes[other] =
                    (0..n).find(|&i| i != self.panes[self.focused_pane]).unwrap_or(self.panes[self.focused_pane]);
            }
            let panes = self.panes;
            let mut focus_to: Option<usize> = None;
            // Set by either pane's header right-click → run after the loop to
            // avoid borrowing self while rendering.
            let mut sync_panes_req = false;
            let mut save_setup_req = false;

            // Manual two-pane split with hard clipping per pane — egui's
            // `columns` doesn't clip, so the wide table bled into the other
            // pane. Each pane gets its own rect, a clip rect, and there's a
            // visible vertical divider between them.
            let full = ui.available_rect_before_wrap();
            let gap = 9.0;
            let half = ((full.width() - gap) / 2.0).max(80.0);
            let rects = [
                egui::Rect::from_min_size(full.min, egui::vec2(half, full.height())),
                egui::Rect::from_min_size(
                    egui::pos2(full.min.x + half + gap, full.min.y),
                    egui::vec2(half, full.height()),
                ),
            ];
            let sep_x = full.min.x + half + gap / 2.0;
            ui.painter().vline(
                sep_x,
                full.min.y..=full.max.y,
                egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.fg_stroke.color),
            );
            // Remember each pane's rect (+ its tab) so a drag can drop onto the
            // other pane, not just the tab header.
            self.pane_rects = vec![(panes[0], rects[0]), (panes[1], rects[1])];

            for (slot, &rect) in rects.iter().enumerate() {
                let tab_idx = panes[slot];
                let focused = tab_idx == self.active_tab;
                ui.allocate_ui_at_rect(rect, |ui| {
                    ui.set_clip_rect(rect); // <- prevents the table from overflowing the pane
                    ui.push_id(("pane", tab_idx), |ui| {
                        let title = self.tab_title(tab_idx);
                        ui.horizontal(|ui| {
                            let resp = if focused {
                                ui.label(RichText::new(format!("● {}", title)).strong())
                            } else {
                                ui.label(
                                    RichText::new(format!("○ {}", title))
                                        .color(Color32::from_gray(150)),
                                )
                            };
                            // Right-click either pane header → sync the two open
                            // folders (the split-view sync the user asked for).
                            resp.context_menu(|ui| {
                                if ui.button("⇄ Diese beiden Ordner synchronisieren").clicked() {
                                    sync_panes_req = true;
                                    ui.close_menu();
                                }
                                if ui.button("＋ Als Sync-Setup speichern…").clicked() {
                                    save_setup_req = true;
                                    ui.close_menu();
                                }
                            });
                        });
                        ui.separator();
                        if focused {
                            self.current_render_tab = tab_idx;
                            self.ui_pane_search(ui);
                            self.ui_table(ui);
                        } else {
                            self.swap_with_tab(tab_idx);
                            self.current_render_tab = tab_idx;
                            self.ui_pane_search(ui);
                            self.band_suppressed = true; // band belongs to the focused pane
                            self.ui_table(ui);
                            self.band_suppressed = false;
                            self.swap_with_tab(tab_idx);
                        }
                        // Click anywhere in this pane focuses it (both panes).
                        let pressed = ui.input(|i| i.pointer.any_pressed());
                        if pressed {
                            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                                if rect.contains(pos) {
                                    focus_to = Some(slot);
                                }
                            }
                        }
                    });
                });
            }
            if let Some(slot) = focus_to {
                self.focused_pane = slot;
                self.switch_tab(panes[slot]);
            }
            // Outline the focused pane so it's obvious which one a top-bar tab
            // selection (and keyboard actions) will apply to.
            let fr = rects[self.focused_pane.min(1)];
            ui.painter().rect_stroke(
                fr.shrink(1.0),
                4.0,
                egui::Stroke::new(2.0, Color32::from_rgb(90, 150, 220)),
            );
            if sync_panes_req {
                self.sync_split_panes();
            }
            if save_setup_req {
                let (_, root_a) = self.pane_backend(panes[0]);
                let (_, root_b) = self.pane_backend(panes[1]);
                self.job_editor = Some(JobEditor::blank(root_a, root_b));
                self.show_sync_jobs = true;
            }
        });
    }

    pub(in crate::app) fn new_tab(&mut self) {
        let cur = self.root_path.clone();
        // A fresh tab has no backend; if the current tab is remote, open the new
        // one at a LOCAL default instead of the (unreachable-without-backend)
        // remote path. The current tab's connection is parked with its TabState
        // by switch_tab and is unaffected.
        let cur_is_remote = self.remote.is_some();
        self.tabs.push(TabState::default());
        let idx = self.tabs.len() - 1;
        self.switch_tab(idx);
        let target = if cur.is_empty() || cur_is_remote {
            self.home.clone()
        } else {
            PathBuf::from(cur.replace('/', std::path::MAIN_SEPARATOR_STR))
        };
        self.start_scan_navigated(target, false);
    }

    pub(in crate::app) fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return;
        }
        if i == self.active_tab {
            let to = if i + 1 < self.tabs.len() { i + 1 } else { i - 1 };
            self.switch_tab(to);
        }
        let t = self.tabs.remove(i);
        if let Some(h) = t.scan_handle {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if self.active_tab > i {
            self.active_tab -= 1;
        }
    }

    /// Compact per-pane name filter/search, shown at the top of each split pane
    /// so the two panes filter independently. Operates on the currently
    /// swapped-in tab's filter (each pane is rendered inside its own swap), and
    /// commits + recomputes immediately for that pane.
    pub(in crate::app) fn ui_pane_search(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("🔍");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.text_draft)
                    .hint_text("Filtern (Name/Regex/Glob)…")
                    .desired_width(f32::INFINITY),
            );
            if resp.changed() {
                self.filter.text = self.text_draft.clone();
                self.recompute_view();
            }
            // Cycle the match mode (substring → regex → glob) so each pane can
            // choose its own.
            let mode_label = match self.filter.text_mode {
                crate::types::TextMode::Substring => "abc",
                crate::types::TextMode::Regex => ".*",
                crate::types::TextMode::Glob => "*?",
            };
            if ui.small_button(mode_label).on_hover_text("Modus: Text / Regex / Glob").clicked() {
                self.filter.text_mode = match self.filter.text_mode {
                    crate::types::TextMode::Substring => crate::types::TextMode::Regex,
                    crate::types::TextMode::Regex => crate::types::TextMode::Glob,
                    crate::types::TextMode::Glob => crate::types::TextMode::Substring,
                };
                self.recompute_view();
            }
            if !self.text_draft.is_empty() && ui.small_button("×").on_hover_text("Filter löschen").clicked() {
                self.text_draft.clear();
                self.filter.text.clear();
                self.recompute_view();
            }
        });
    }

    pub(in crate::app) fn tab_title(&self, i: usize) -> String {
        // Per-tab path + connection (active tab's live in the App fields).
        let (p, remote_label, is_share) = if i == self.active_tab {
            (
                &self.root_path,
                self.remote.as_ref().map(|r| r.label.as_str()),
                self.net_conn.is_some(),
            )
        } else {
            let t = &self.tabs[i];
            (
                &t.root_path,
                t.remote.as_ref().map(|r| r.label.as_str()),
                t.net_conn.is_some(),
            )
        };
        if p.is_empty() && remote_label.is_none() {
            return "Neuer Tab".to_string();
        }
        let t = p.trim_end_matches('/');
        let base = t.rsplit('/').next().unwrap_or(t);
        let base = if base.is_empty() { t } else { base };

        // Remote/share tabs get a marker + the connection name, so they're
        // identifiable (the bare folder name isn't enough).
        let title = if let Some(label) = remote_label {
            // "sftp://user@host:port" -> "user@host:port"
            let host = label.split("://").nth(1).unwrap_or(label);
            format!("🌐 {host} · {base}")
        } else if is_share {
            format!("🖧 {base}")
        } else {
            base.to_string()
        };

        if title.chars().count() > 24 {
            let mut out: String = title.chars().take(23).collect();
            out.push('…');
            out
        } else {
            title
        }
    }

    pub(in crate::app) fn ui_tabbar(&mut self, ui: &mut egui::Ui) {
        enum TabAction {
            Switch(usize),
            Close(usize),
            New,
        }
        let mut action: Option<TabAction> = None;
        let dragging = self.drag_active;
        let mut header_rects: Vec<(usize, egui::Rect)> = Vec::new();
        ui.horizontal(|ui| {
            for i in 0..self.tabs.len() {
                let selected = i == self.active_tab;
                let title = self.tab_title(i);
                // Prefix the first nine tabs with their Alt+N accelerator.
                let label = if i < 9 {
                    format!("{}·{}", i + 1, title)
                } else {
                    title
                };
                let mut resp = ui.selectable_label(selected, label);
                if i < 9 {
                    resp = resp.on_hover_text(format!("Alt+{} — zu diesem Tab", i + 1));
                }
                header_rects.push((i, resp.rect));
                // Highlight a tab as a drop target while files are being dragged
                // from another tab.
                if dragging && i != self.drag_source_tab && resp.hovered() {
                    ui.painter().rect_stroke(
                        resp.rect.expand(1.0),
                        3.0,
                        egui::Stroke::new(2.0, Color32::from_rgb(120, 200, 255)),
                    );
                }
                if resp.clicked() && !selected {
                    action = Some(TabAction::Switch(i));
                }
                if resp.middle_clicked() {
                    action = Some(TabAction::Close(i));
                }
                if selected && self.tabs.len() > 1 {
                    if ui
                        .small_button("✕")
                        .on_hover_text("Tab schließen (Ctrl+W)")
                        .clicked()
                    {
                        action = Some(TabAction::Close(i));
                    }
                }
            }
            let r = ui.button("＋").on_hover_text("Neuer Tab (Ctrl+T)");
            self.accel_push('T', r.rect, AccelAct::NewTab);
            if r.clicked() {
                action = Some(TabAction::New);
            }
        });
        self.tab_header_rects = header_rects;
        match action {
            Some(TabAction::Switch(i)) => self.switch_tab(i),
            Some(TabAction::Close(i)) => self.close_tab(i),
            Some(TabAction::New) => self.new_tab(),
            None => {}
        }
    }

    // ─── Scanning / navigation ──────────────────────────────────────────

}
