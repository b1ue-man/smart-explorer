//! Renaming and moving inside the vault. A folder keeps its directory ID,
//! so only its node moves (its content stays where it is). A destination
//! that exists is refused (`rename_overwrites` is false); the storage's
//! no-replace operation is the gate wherever it exists.
//!
//! Between short (`<base64>.c9r`) and shortened (`<digest>.c9s/`) names the
//! type file (`contents.c9r` of a long file, `dir.c9r`, `symlink.c9r`) moves
//! into a new node folder created exclusively, then the old node folder is
//! removed. A folder that is only the display name of a conflict copy is
//! never moved: it shares its ID with another folder (C1.8).
use std::io;

use crate::cryptomator::{CONTENTS_FILE, DIR_FILE, INFLATED_NAME_FILE, SYMLINK_FILE};
use crate::vault::core::capabilities::long_names_no_replace;
use crate::vault::core::path_map::key_within;
use crate::vfs::Backend;

use super::backend::VaultBackend;
use super::backend_nodes::{invalid_input, Found, NodeKind, NodeRef, Parts};

fn already_exists() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Eintrag im Tresor existiert bereits",
    )
}

impl VaultBackend {
    /// `rename` (`no_replace = false`) and `rename_no_replace`.
    pub(super) fn rename_entry(&self, src: &str, dst: &str, no_replace: bool) -> io::Result<()> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let from = Parts::of(src)?
                .ok_or_else(|| invalid_input("Die Tresorwurzel lässt sich nicht umbenennen"))?;
            let to =
                Parts::of(dst)?.ok_or_else(|| invalid_input("Die Tresorwurzel ist kein Ziel"))?;
            let (from_key, to_key) = (from.key(), to.key());
            if from_key == to_key {
                return Ok(());
            }
            if key_within(&to_key, &from_key) {
                return Err(invalid_input(
                    "Ein Ordner lässt sich nicht in sich selbst verschieben",
                ));
            }
            let found = self.find_at(&from)?;
            if found.node.alias && matches!(found.kind, NodeKind::Dir { .. }) {
                return Err(invalid_input(
                    "Konfliktkopie eines Ordners: über „Tresor prüfen“ trennen",
                ));
            }
            let parent = self.dir_for(&to.parent)?;
            let to_parent_key = to.parent_key();
            match self.find(&to_parent_key, &parent, to.name) {
                Ok(_) => return Err(already_exists()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let target = self.canonical(&parent, to.name)?;
            if no_replace
                && target.shortened
                && !long_names_no_replace(self.inner.staged_write_capabilities(&self.root))
            {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Umbenennen ohne Ersetzen auf einen langen Namen braucht atomares Anlegen",
                ));
            }
            let moved = self.move_node(&found, &target, no_replace);
            self.cache.forget_tree(&from_key);
            self.cache.forget_aliases(&from.parent_key());
            self.cache.forget_aliases(&to_parent_key);
            moved
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// One storage rename that never replaces: the storage's no-replace
    /// rename, for `rename` falling back to the plain one where the storage
    /// has none (the destination was checked to be absent just before).
    fn rename_stored(&self, from: &str, to: &str, no_replace: bool) -> io::Result<()> {
        let (from, to) = (self.at(from), self.at(to));
        match self.inner.rename_no_replace(&from, &to) {
            Err(error) if error.kind() == io::ErrorKind::Unsupported && !no_replace => {
                self.inner.rename(&from, &to)
            }
            other => other,
        }
    }

    fn move_node(&self, found: &Found, target: &NodeRef, no_replace: bool) -> io::Result<()> {
        let source = &found.node;
        if !source.shortened && !target.shortened {
            // A file or a whole node folder: one rename.
            return self.rename_stored(&source.node(), &target.node(), no_replace);
        }
        let (type_file, file) = match found.kind {
            NodeKind::File { .. } => (CONTENTS_FILE, true),
            NodeKind::Dir { .. } => (DIR_FILE, false),
            NodeKind::Symlink { .. } => (SYMLINK_FILE, false),
        };
        // A short file is its own content; everything else has a folder.
        let source_has_folder = source.shortened || !file;
        let target_has_folder = target.shortened || !file;
        let created = if target_has_folder {
            self.inner.create_dir_new(&self.at(&target.node()))?;
            if target.shortened {
                if let Err(error) = self.ensure_long_name(target) {
                    self.remove_quietly(&[target.inside(INFLATED_NAME_FILE)], &target.node());
                    return Err(error);
                }
            }
            true
        } else {
            false
        };
        let from = if source_has_folder {
            source.inside(type_file)
        } else {
            source.node()
        };
        let to = if target_has_folder {
            target.inside(type_file)
        } else {
            target.node()
        };
        if let Err(error) = self.rename_stored(&from, &to, no_replace) {
            if created {
                self.remove_quietly(&[target.inside(INFLATED_NAME_FILE)], &target.node());
            }
            return Err(error);
        }
        if source_has_folder {
            // The entry lives on under its new name; an old node folder that
            // cannot be removed is a leftover „Tresor prüfen“ reports.
            self.remove_quietly(&[source.inside(INFLATED_NAME_FILE)], &source.node());
        }
        Ok(())
    }
}
