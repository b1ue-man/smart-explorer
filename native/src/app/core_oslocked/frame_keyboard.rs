use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn update_keyboard(&mut self, ctx: &egui::Context) {
        // ─── Alt key-overlay (accelerators) ────────────────────────────
        // While the overlay is up, a bare letter/digit fires its control (using
        // last frame's rects — controls don't move) and closes the overlay; Esc
        // closes it. Runs before the other shortcuts so the key is consumed
        // before type-to-jump or a text field sees it.
        if self.accel_mode {
            let targets = self.accel_all();
            let hit = ctx.input_mut(|i| {
                use egui::{Key, Modifiers};
                if i.consume_key(Modifiers::NONE, Key::Escape) {
                    return Some(None);
                }
                for (c, _rect, act) in &targets {
                    if let Some(k) = accel_key(*c) {
                        if i.consume_key(Modifiers::NONE, k) {
                            return Some(Some(*act));
                        }
                    }
                }
                None
            });
            if let Some(act) = hit {
                self.accel_mode = false;
                if let Some(a) = act {
                    self.exec_accel(a);
                }
            }
        }

        // ─── Global keyboard shortcuts ─────────────────────────────────
        // `wants_keyboard_input` = a text field has focus; table shortcuts
        // and type-to-jump must not fire then.
        let typing = ctx.wants_keyboard_input();
        let renaming = self.rename_open.is_some();
        let mut acts: Vec<KbdAct> = Vec::new();
        let mut jump_text = String::new();
        // Clipboard ops are driven by egui's semantic Copy/Cut/Paste events
        // (and key-combos as a fallback) — see the event scan below.
        let mut do_copy = false;
        let mut do_cut = false;
        let mut do_paste = false;

        ctx.input_mut(|i| {
            use egui::{Key, Modifiers};

            // Tab management & global navigation (work even while typing)
            if i.consume_key(Modifiers::COMMAND, Key::T) {
                acts.push(KbdAct::NewTab);
            }
            if i.consume_key(Modifiers::COMMAND, Key::W) {
                acts.push(KbdAct::CloseTab);
            }
            if i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) {
                acts.push(KbdAct::PrevTab);
            }
            if i.consume_key(Modifiers::CTRL, Key::Tab) {
                acts.push(KbdAct::NextTab);
            }
            if i.consume_key(Modifiers::COMMAND, Key::L) {
                acts.push(KbdAct::PathEdit);
            }
            if i.consume_key(Modifiers::NONE, Key::F5) {
                acts.push(KbdAct::Rescan);
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowLeft) {
                acts.push(KbdAct::Back);
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowRight) {
                acts.push(KbdAct::Forward);
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowUp) {
                acts.push(KbdAct::Up);
            }
            // Focus jumps + help work even while a field is focused.
            if i.consume_key(Modifiers::NONE, Key::F1) {
                acts.push(KbdAct::ToggleHelp);
            }
            if i.consume_key(Modifiers::NONE, Key::F6) {
                acts.push(KbdAct::ToggleSplit);
            }
            // Ctrl+F focuses the active tab's name filter; Ctrl+Shift+F the
            // sidebar's global folder search.
            if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F) {
                acts.push(KbdAct::FocusSearch);
            }
            if i.consume_key(Modifiers::COMMAND, Key::F) {
                acts.push(KbdAct::FocusFilter);
            }
            if i.consume_key(Modifiers::NONE, Key::F3) {
                acts.push(KbdAct::FocusFilter);
            }
            // Alt+1..9 → jump to that tab (Alt+9 = last). Works while typing,
            // like the other tab shortcuts above.
            for (n, key) in [
                Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7,
                Key::Num8, Key::Num9,
            ]
            .into_iter()
            .enumerate()
            {
                if i.consume_key(Modifiers::ALT, key) {
                    acts.push(KbdAct::SelectTab(n));
                }
            }

            if !typing && !renaming {
                if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::N) {
                    acts.push(KbdAct::NewFolder);
                }
                let copy_paths = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::C);
                if copy_paths {
                    acts.push(KbdAct::CopyPathsText);
                }
                if i.consume_key(Modifiers::COMMAND, Key::A) {
                    acts.push(KbdAct::SelectAll);
                }
                // Ctrl+C / Ctrl+X / Ctrl+V do NOT arrive as Key events — the
                // winit backend turns them into semantic Copy/Cut/Paste events
                // (so text widgets work). consume_key on V/C/X therefore never
                // matches; we read the semantic events instead. The key-combo
                // checks below are kept only as a belt-and-braces fallback for
                // backends that DO emit them.
                for ev in &i.events {
                    match ev {
                        egui::Event::Copy => do_copy = true,
                        egui::Event::Cut => do_cut = true,
                        egui::Event::Paste(_) => do_paste = true,
                        _ => {}
                    }
                }
                if i.consume_key(Modifiers::COMMAND, Key::C) {
                    do_copy = true;
                }
                if i.consume_key(Modifiers::COMMAND, Key::X) {
                    do_cut = true;
                }
                if i.consume_key(Modifiers::COMMAND, Key::V) {
                    do_paste = true;
                }
                // Ctrl+Shift+C means "copy paths as text" — don't also fire the
                // file copy from the Event::Copy the backend emits for it.
                if copy_paths {
                    do_copy = false;
                }
                if i.consume_key(Modifiers::COMMAND, Key::R) {
                    acts.push(KbdAct::ToggleRecursive);
                }
                if i.consume_key(Modifiers::SHIFT, Key::Delete) {
                    acts.push(KbdAct::PermanentDelete);
                }
                if i.consume_key(Modifiers::NONE, Key::Delete) {
                    acts.push(KbdAct::TrashSel);
                }
                if i.consume_key(Modifiers::NONE, Key::Escape) {
                    acts.push(KbdAct::ClearSel);
                }
                if i.consume_key(Modifiers::NONE, Key::F2) {
                    acts.push(KbdAct::RenameSel);
                }
                if i.consume_key(Modifiers::ALT, Key::Enter) {
                    acts.push(KbdAct::Properties);
                }
                if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::E) {
                    acts.push(KbdAct::RevealInExplorer);
                }
                if i.consume_key(Modifiers::COMMAND, Key::I) {
                    acts.push(KbdAct::InvertSelection);
                }
                if i.consume_key(Modifiers::COMMAND, Key::B) {
                    acts.push(KbdAct::StarCurrent);
                }
                if i.consume_key(Modifiers::NONE, Key::Backspace) {
                    acts.push(KbdAct::Up);
                }
                if i.consume_key(Modifiers::NONE, Key::Enter) {
                    acts.push(KbdAct::Open);
                }
                for shift in [false, true] {
                    let m = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
                    if i.consume_key(m, Key::ArrowDown) {
                        acts.push(KbdAct::Move(1, shift));
                    }
                    if i.consume_key(m, Key::ArrowUp) {
                        acts.push(KbdAct::Move(-1, shift));
                    }
                    if i.consume_key(m, Key::PageDown) {
                        acts.push(KbdAct::Move(15, shift));
                    }
                    if i.consume_key(m, Key::PageUp) {
                        acts.push(KbdAct::Move(-15, shift));
                    }
                    if i.consume_key(m, Key::Home) {
                        acts.push(KbdAct::MoveToEnd(false, shift));
                    }
                    if i.consume_key(m, Key::End) {
                        acts.push(KbdAct::MoveToEnd(true, shift));
                    }
                }
                // Type-to-jump: collect plain text events
                for ev in &i.events {
                    if let egui::Event::Text(t) = ev {
                        jump_text.push_str(t);
                    }
                }
            }
        });
        // A mouse click means the user took manual control of the list, so a
        // later Enter-on-folder should not bounce back to the filter, and the
        // Alt overlay (if any) closes.
        if ctx.input(|i| i.pointer.any_pressed()) {
            self.search_nav_from_filter = false;
            self.accel_mode = false;
        }
        // Alt key-overlay toggle: a clean Alt tap (pressed and released with no
        // other key or click) toggles the overlay; using Alt as a modifier
        // (Alt+←, Alt+1, …) does not. egui exposes Alt only as a modifier flag,
        // so detect it via the state transition.
        let (alt_now, key_or_click) = ctx.input(|i| {
            (
                i.modifiers.alt,
                i.pointer.any_pressed()
                    || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })),
            )
        });
        if alt_now && !self.alt_prev {
            self.alt_dirty = false; // Alt just went down
        }
        if alt_now && key_or_click {
            self.alt_dirty = true; // used together with another input
        }
        if !alt_now && self.alt_prev && !self.alt_dirty {
            self.accel_mode = !self.accel_mode; // clean tap released
        }
        self.alt_prev = alt_now;

        for act in acts {
            match act {
                KbdAct::SelectAll => self.select_all(),
                KbdAct::CopyPathsText => self.copy_paths_to_clipboard(ctx),
                KbdAct::TrashSel => self.trash_selected(),
                KbdAct::ClearSel => self.selection.clear(),
                KbdAct::Rescan => self.rescan(),
                KbdAct::Back => self.navigate_back(),
                KbdAct::Forward => self.navigate_forward(),
                KbdAct::Up => self.navigate_up(),
                KbdAct::ToggleRecursive => {
                    self.recursive = !self.recursive;
                    self.rescan();
                }
                KbdAct::NewTab => self.new_tab(),
                KbdAct::CloseTab => self.close_tab(self.active_tab),
                KbdAct::NextTab => {
                    let n = self.tabs.len();
                    if n > 1 {
                        self.switch_tab((self.active_tab + 1) % n);
                    }
                }
                KbdAct::PrevTab => {
                    let n = self.tabs.len();
                    if n > 1 {
                        self.switch_tab((self.active_tab + n - 1) % n);
                    }
                }
                KbdAct::NewFolder => self.create_new_folder(),
                KbdAct::RenameSel => self.open_rename(),
                KbdAct::PathEdit => {
                    self.path_edit_mode = true;
                    self.path_edit_focus = true;
                }
                KbdAct::Move(d, shift) => self.move_cursor(d, shift),
                KbdAct::MoveToEnd(to_end, shift) => {
                    if !self.view.is_empty() {
                        let pos = if to_end { self.view.len() - 1 } else { 0 };
                        self.move_cursor_to(pos, shift);
                    }
                }
                KbdAct::Open => {
                    // Enter-to-open. If this drills into a FOLDER during a
                    // filter-driven nav session, bounce focus back to the filter
                    // (cleared) so the user can type the next path segment without
                    // touching the mouse. Files end the session.
                    let into_folder = self.selection_single_dir();
                    self.open_selection();
                    if self.search_nav_from_filter && into_folder {
                        self.text_draft.clear();
                        self.filter.text.clear();
                        self.recompute_view();
                        self.name_filter_focus = true;
                        self.show_filters = true;
                    } else {
                        self.search_nav_from_filter = false;
                    }
                }
                KbdAct::Properties => self.show_properties(),
                KbdAct::PermanentDelete => self.delete_permanent(),
                KbdAct::RevealInExplorer => {
                    if let Some(p) = self.focus_path() {
                        self.open_in_explorer(&p);
                    }
                }
                KbdAct::InvertSelection => self.invert_selection(),
                KbdAct::FocusSearch => {
                    // Ctrl+F = folder search → drop straight into `/`-mode so the
                    // dropdown owns the keyboard. Carry a plain filter over as the
                    // search query; leave an existing `/`-search untouched.
                    self.show_filters = true;
                    if omni_mode(&self.text_draft) != OmniMode::FolderSearch {
                        let carry = if omni_mode(&self.text_draft) == OmniMode::Filter {
                            self.text_draft.trim().to_string()
                        } else {
                            String::new()
                        };
                        self.text_draft = format!("/{}", carry);
                        self.filter.text.clear();
                        self.recompute_view();
                        if !carry.is_empty() {
                            self.folder_search_query = carry;
                            self.folder_search_pending_at = Some(std::time::Instant::now());
                        }
                    }
                    self.folder_search_focus = true;
                    self.search_nav_from_filter = false;
                }
                KbdAct::FocusFilter => {
                    self.show_filters = true;
                    self.name_filter_focus = true;
                    // Fresh filter session: we're in the filter, not the list.
                    self.search_nav_from_filter = false;
                }
                KbdAct::ToggleHelp => self.show_help = !self.show_help,
                KbdAct::ToggleSplit => self.toggle_split(),
                KbdAct::StarCurrent => self.star_current_folder(),
                KbdAct::SelectTab(n) => {
                    // Alt+9 = last tab; otherwise the Nth tab if it exists.
                    let target = if n == 8 {
                        self.tabs.len().saturating_sub(1)
                    } else {
                        n
                    };
                    if target < self.tabs.len() {
                        self.switch_tab(target);
                    }
                }
            }
        }
        if !jump_text.is_empty() {
            self.type_to_jump(&jump_text);
        }
        // Omnibox Enter / dropdown-row activation (captured in `ui_filterbar`),
        // processed now that the frame's view + folder-search hits have settled.
        if std::mem::take(&mut self.filter_enter) {
            self.handle_omni_enter(ctx);
        }
        if let Some(a) = self.omni_activate.take() {
            self.execute_omni(a, ctx);
        }

        // Drain the background clipboard-key poller (Windows). This is what
        // actually makes Ctrl+V work for a file clipboard — see clip_key_rx.
        #[cfg(windows)]
        if !typing && !renaming {
            if let Some(rx) = self.clip_key_rx.as_ref() {
                while let Ok(k) = rx.try_recv() {
                    match k {
                        ClipKey::Copy => do_copy = true,
                        ClipKey::Cut => do_cut = true,
                        ClipKey::Paste => do_paste = true,
                    }
                }
            }
        }

        // File-clipboard ops, triggered by egui's semantic Copy/Cut/Paste
        // events, the OS-level key poller above, or the key-combo fallback.
        if do_copy {
            self.clipboard_copy_files(false);
        }
        if do_cut {
            self.clipboard_copy_files(true);
        }
        if do_paste {
            self.clipboard_paste_files();
        }

    }
}
