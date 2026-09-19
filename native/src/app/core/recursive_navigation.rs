use super::prelude::*;
use super::*;

impl App {
    pub(super) fn toggle_recursive_folder(&mut self, index: usize) {
        if !self.recursive || !self.entries[index].is_dir {
            return;
        }
        let key = self.entries[index].key();
        if !self.tree.collapsed.insert(key.clone()) {
            self.tree.collapsed.remove(&key);
        }
        self.view = self.tree.displayed(&self.entries);
        // Keep selected descendants, but move a now-hidden keyboard cursor to
        // the folder so arrow navigation continues at a visible position.
        if self.cursor.as_ref().is_some_and(|cursor| {
            !self
                .view
                .iter()
                .any(|&(i, _)| self.entries[i].path == *cursor)
        }) {
            self.cursor = Some(self.entries[index].path.clone());
        }
        self.band_press = None;
        self.band_active = false;
    }

    pub(super) fn recursive_arrow(&mut self, expand: bool) {
        if !self.recursive {
            return;
        }
        let Some(row) = self.cursor.as_ref().and_then(|cursor| {
            self.view
                .iter()
                .position(|&(i, _)| self.entries[i].path == *cursor)
        }) else {
            return;
        };
        let (index, depth) = self.view[row];
        let entry = &self.entries[index];
        if entry.is_dir && self.tree.collapsed.contains(&entry.key()) == expand {
            self.toggle_recursive_folder(index);
        } else if expand {
            if self
                .view
                .get(row + 1)
                .is_some_and(|&(_, child_depth)| child_depth > depth)
            {
                self.move_cursor_to(row + 1, false);
            }
        } else if let Some(parent) = (0..row).rev().find(|&i| self.view[i].1 < depth) {
            self.move_cursor_to(parent, false);
        }
    }
}
