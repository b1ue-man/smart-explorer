//! Renaming a desktop file in place (F2, menu "Umbenennen"; DSK-M4): an
//! editor over the tile's label with the name selected up to its extension.
//! Enter or leaving the field renames, Esc cancels; an unchanged or empty
//! name changes nothing. The name is checked again before the file system
//! sees it (`desktop_ops::check_name`).
use eframe::egui;

use super::desktop_surface_tile_ui::label_rect;
use crate::desktop::core::layout::ItemId;

/// The rename in progress.
pub(crate) struct Rename {
    pub(crate) item: ItemId,
    name: String,
    text: String,
    selected: bool,
}

/// What the editor decided this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RenameOutcome {
    Editing,
    Cancel,
    Commit { from: String, to: String },
}

impl Rename {
    pub(crate) fn new(item: ItemId, name: &str) -> Rename {
        Rename {
            item,
            name: name.to_string(),
            text: name.to_string(),
            selected: false,
        }
    }

    /// Characters of the name before its extension (all of a folder-like
    /// name without a dot, all of a name that starts with its only dot).
    fn stem_chars(&self) -> usize {
        match self.text.rfind('.') {
            Some(dot) if dot > 0 => self.text[..dot].chars().count(),
            _ => self.text.chars().count(),
        }
    }

    /// The editor over the label of the tile at `tile`.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, tile: egui::Rect) -> RenameOutcome {
        let id = egui::Id::new("se-desktop-rename");
        let editor = egui::TextEdit::singleline(&mut self.text)
            .id(id)
            .horizontal_align(egui::Align::Center);
        let response = ui.put(label_rect(tile), editor);
        if !self.selected {
            response.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let range = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(self.stem_chars()),
                );
                state.cursor.set_char_range(Some(range));
                egui::TextEdit::store_state(ui.ctx(), id, state);
                self.selected = true;
            }
            return RenameOutcome::Editing;
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            return RenameOutcome::Cancel;
        }
        if response.lost_focus() {
            let to = self.text.trim().to_string();
            if to.is_empty() || to == self.name {
                return RenameOutcome::Cancel;
            }
            return RenameOutcome::Commit {
                from: self.name.clone(),
                to,
            };
        }
        RenameOutcome::Editing
    }
}
