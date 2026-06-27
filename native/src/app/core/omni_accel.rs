use super::prelude::*;
use super::*;

impl App {
    /// Commit the debounced name/extension drafts into the active filter and
    /// rebuild the view. Shared by the keystroke debounce and Enter (which must
    /// flush immediately so the result count is current). Only plain filter-mode
    /// text narrows the listing — a typed path or `>command` must leave the
    /// current folder's entries visible.
    pub(in crate::app) fn flush_text_filter(&mut self) {
        self.filter.text = if omni_mode(&self.text_draft) == OmniMode::Filter {
            self.text_draft.clone()
        } else {
            String::new()
        };
        self.filter.extensions = self
            .ext_draft
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(|s| s.trim().trim_start_matches('.').to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        self.filter_pending_at = None;
        self.recompute_view();
    }

    /// Enter pressed in the active tab's name filter — the heart of cursorless
    /// navigation. Flush the filter so the view is current, then branch on the
    /// number of matches:
    ///  - 0 → stay in the filter (nothing to do);
    ///  - 1 → open it. A folder is entered, the filter cleared, and focus kept
    ///    on the filter so the next segment can be typed straight away; a file
    ///    is simply opened;
    ///  - multiple matches → hand keyboard focus to the result list (cursor on
    ///    the first row) so arrow keys navigate and Enter there opens — and,
    ///    for a folder, bounces back here (see the `Open` handler).
    pub(in crate::app) fn handle_filter_enter(&mut self) {
        self.flush_text_filter();
        let n = self.view.len();
        if n == 0 {
            return;
        }
        if n == 1 {
            let e = &self.entries[self.view[0].0];
            let is_dir = e.is_dir;
            let path = e.path.to_string();
            let name = e.name.to_string();
            let id = e.id.as_ref().map(|s| s.to_string());
            if is_dir {
                self.start_scan(PathBuf::from(
                    path.replace('/', std::path::MAIN_SEPARATOR_STR),
                ));
                self.text_draft.clear();
                self.filter.text.clear();
                self.recompute_view();
            } else {
                self.open_file(path, name, id, OpenMode::Default);
            }
            self.name_filter_focus = true;
            self.search_nav_from_filter = false;
        } else {
            // Multiple hits: move into the list for arrow-key navigation.
            self.move_cursor_to(0, false);
            self.search_nav_from_filter = true;
        }
    }

    // ─── Alt key-overlay (accelerators) ─────────────────────────────────────

    /// Register a control for the Alt overlay (only while it's showing).
    pub(in crate::app) fn accel_push(&mut self, c: char, rect: egui::Rect, act: AccelAct) {
        if self.accel_mode {
            self.accel_targets.push((c, rect, act));
        }
    }

    /// All overlay targets this frame: the registered toolbar controls plus a
    /// digit per visible tab (1..9).
    pub(in crate::app) fn accel_all(&self) -> Vec<(char, egui::Rect, AccelAct)> {
        let mut v = self.accel_targets.clone();
        for (i, rect) in &self.tab_header_rects {
            if *i < 9 {
                v.push(((b'1' + *i as u8) as char, *rect, AccelAct::Tab(*i)));
            }
        }
        v
    }

    pub(in crate::app) fn exec_accel(&mut self, act: AccelAct) {
        match act {
            AccelAct::Back => self.navigate_back(),
            AccelAct::Forward => self.navigate_forward(),
            AccelAct::Up => self.navigate_up(),
            AccelAct::PickFolder => {
                let init = self.root_path.clone();
                self.open_picker(PickerPurpose::ScanFolder, &init);
            }
            AccelAct::Split => self.toggle_split(),
            AccelAct::NewTab => self.new_tab(),
            AccelAct::Tab(i) => self.switch_tab(i),
        }
    }

    /// Dim the window and draw the accelerator badges over registered controls.
    pub(in crate::app) fn draw_accel_overlay(&self, ctx: &egui::Context) {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("accel_overlay"),
        ));
        painter.rect_filled(ctx.screen_rect(), 0.0, Color32::from_black_alpha(110));
        for (c, rect, _) in self.accel_all() {
            draw_accel_badge(&painter, rect, c);
        }
    }

    // ─── Omnibox (combo-field) ──────────────────────────────────────────────

    /// Enter in the omnibox. A highlighted dropdown row wins; otherwise act by
    /// mode: run the first command (`>`), navigate the typed path / `..`, or
    /// fall through to the in-list cursorless navigation (plain filter text).
    pub(in crate::app) fn handle_omni_enter(&mut self, ctx: &egui::Context) {
        if let Some(sel) = self.omni_sel {
            let items = self.build_omni_items();
            if let Some(it) = items.get(sel) {
                let action = it.action.clone();
                self.execute_omni(action, ctx);
                return;
            }
        }
        let raw = self.text_draft.clone();
        match omni_mode(&raw) {
            OmniMode::Command | OmniMode::FolderSearch => {
                // No row highlighted → take the best match (top dropdown row).
                if let Some(it) = self.build_omni_items().into_iter().next() {
                    self.execute_omni(it.action, ctx);
                } else {
                    self.clear_omni();
                }
            }
            OmniMode::Path => {
                if let Some(n) = omni_up_levels(&raw) {
                    self.navigate_up_n(n);
                } else {
                    let p = expand_omni_path(&raw, &self.home, &self.root_path);
                    if !p.is_empty() {
                        self.start_scan(PathBuf::from(p));
                    }
                }
                self.clear_omni();
            }
            OmniMode::Filter => self.handle_filter_enter(),
        }
    }

    /// Build the dropdown rows for the current omnibox text, by mode:
    ///  - Command (`>`): folder-action commands + navigation/root targets;
    ///  - Path: root targets filtered by the typed text (Enter still navigates
    ///    the full typed path / `..`);
    ///  - FolderSearch (`/`): fuzzy global folder-jump hits + matching roots;
    ///  - Filter: NO dropdown — plain text just narrows the list, so the arrow
    ///    keys stay with the file list.
    pub(in crate::app) fn build_omni_items(&self) -> Vec<OmniItem> {
        let raw = self.text_draft.as_str();
        let mut items: Vec<OmniItem> = Vec::new();
        match omni_mode(raw) {
            OmniMode::Command => {
                let q = raw.trim_start().trim_start_matches('>').trim();
                let cmds: &[(&str, &str, OmniCmd)] = &[
                    ("＋", "Neuer Ordner", OmniCmd::NewFolder),
                    ("🗗", "Im Explorer anzeigen", OmniCmd::Reveal),
                    ("▶", "Terminal hier öffnen", OmniCmd::Terminal),
                    ("⧉", "Pfad kopieren", OmniCmd::CopyPath),
                    ("★", "Favorit umschalten", OmniCmd::StarToggle),
                    ("⟳", "Aktualisieren", OmniCmd::Refresh),
                    ("📊", "Speicher-Analyse", OmniCmd::Analytics),
                ];
                for (icon, label, cmd) in cmds {
                    if fuzzy_contains(label, q) {
                        items.push(OmniItem {
                            icon,
                            label: (*label).to_string(),
                            sub: "Befehl".into(),
                            action: OmniAction::Cmd(*cmd),
                        });
                    }
                }
                self.push_root_items(&mut items, q);
            }
            OmniMode::Path => {
                self.push_root_items(&mut items, raw.trim());
            }
            OmniMode::FolderSearch => {
                let q = raw.trim_start().trim_start_matches('/').trim();
                for (p, _score) in self.folder_search_results.iter().take(30) {
                    items.push(OmniItem {
                        icon: "📁",
                        label: p.clone(),
                        sub: p.clone(),
                        action: OmniAction::Go(p.clone()),
                    });
                }
                // Also offer roots / drives / remotes / favourites that match.
                self.push_root_items(&mut items, q);
            }
            OmniMode::Filter => {}
        }
        items
    }

    /// Append root targets — Home, drives, saved remotes, favorites — keeping
    /// those that fuzzy-match `q`.
    pub(in crate::app) fn push_root_items(&self, items: &mut Vec<OmniItem>, q: &str) {
        let home = self.home.to_string_lossy().to_string();
        if fuzzy_contains("Persönlicher Ordner", q) || fuzzy_contains(&home, q) {
            items.push(OmniItem {
                icon: "🏠",
                label: "Persönlicher Ordner".into(),
                sub: home.clone(),
                action: OmniAction::Go(home),
            });
        }
        for d in &self.drives {
            let trimmed = d.trim_end_matches(['\\', '/']).to_string();
            if fuzzy_contains(&trimmed, q) {
                items.push(OmniItem {
                    icon: "💽",
                    label: format!("Laufwerk {}", trimmed),
                    sub: d.clone(),
                    action: OmniAction::Go(d.clone()),
                });
            }
        }
        for (i, c) in self.saved_connections.iter().enumerate() {
            let label = if c.label.trim().is_empty() {
                c.host.clone()
            } else {
                c.label.clone()
            };
            let sub = if c.user.trim().is_empty() {
                c.host.clone()
            } else {
                format!("{}@{}", c.user, c.host)
            };
            if fuzzy_contains(&label, q) || fuzzy_contains(&sub, q) {
                items.push(OmniItem {
                    icon: "🌐",
                    label,
                    sub,
                    action: OmniAction::Connect(i),
                });
            }
        }
        for f in self.favorites.iter().take(20) {
            let base = f.rsplit('/').next().unwrap_or(f).to_string();
            if fuzzy_contains(&base, q) || fuzzy_contains(f, q) {
                items.push(OmniItem {
                    icon: "★",
                    label: base,
                    sub: f.clone(),
                    action: OmniAction::Go(f.clone()),
                });
            }
        }
    }

    /// Run a dropdown row's action, then clear the omnibox.
    pub(in crate::app) fn execute_omni(&mut self, action: OmniAction, ctx: &egui::Context) {
        match action {
            OmniAction::Go(p) => self.navigate_to_location(&p),
            OmniAction::Connect(i) => {
                if let Some(c) = self.saved_connections.get(i).cloned() {
                    self.connect_saved(&c);
                }
            }
            OmniAction::Cmd(cmd) => match cmd {
                OmniCmd::NewFolder => self.create_new_folder(),
                OmniCmd::Reveal => {
                    if let Some(p) = self.focus_path() {
                        self.open_in_explorer(&p);
                    } else if !self.root_path.is_empty() {
                        self.open_in_explorer(&self.root_path.clone());
                    }
                }
                OmniCmd::Terminal => self.open_terminal_here(),
                OmniCmd::CopyPath => {
                    if !self.root_path.is_empty() {
                        ctx.copy_text(self.root_path.clone());
                    }
                }
                OmniCmd::StarToggle => self.star_current_folder(),
                OmniCmd::Refresh => self.rescan(),
                OmniCmd::Analytics => self.show_analytics = true,
            },
        }
        self.clear_omni();
    }

    /// Reset the omnibox after an action: clear text, filter, and dropdown.
    pub(in crate::app) fn clear_omni(&mut self) {
        self.text_draft.clear();
        self.filter.text.clear();
        self.folder_search_query.clear();
        self.folder_search_results.clear();
        self.omni_sel = None;
        self.recompute_view();
    }

    /// Navigate up `n` folder levels from the current root.
    pub(in crate::app) fn navigate_up_n(&mut self, n: usize) {
        let mut p = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        for _ in 0..n.max(1) {
            match p.parent() {
                Some(par) if !par.as_os_str().is_empty() => p = par.to_path_buf(),
                _ => break,
            }
        }
        if !p.as_os_str().is_empty() && p.to_string_lossy() != self.root_path {
            self.start_scan(p);
        }
    }

    /// Open a system terminal in the current folder.
    pub(in crate::app) fn open_terminal_here(&self) {
        open_terminal_at(&self.root_path);
    }

    /// Open one entry by index: navigate into a folder, or open a file.
    pub(in crate::app) fn activate_entry(&mut self, idx: usize) {
        if idx >= self.entries.len() {
            return;
        }
        let e = &self.entries[idx];
        if e.is_dir {
            let p = PathBuf::from(e.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            self.start_scan(p);
            return;
        }
        let (path, name) = (e.path.to_string(), e.name.to_string());
        let id = e.id.as_ref().map(|s| s.to_string());
        self.open_file(path, name, id, OpenMode::Default);
    }
}
