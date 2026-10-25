//! Folders of the vault: metadata, creating folders (content directory and
//! `dirid.c9r` before the node, so observers never see a node without
//! content, C2.6) and removing empty ones (node first, then content). The
//! listing itself is in `backend_listing.rs`.
use std::io;

use crate::cryptomator::{
    cleartext_file_size, content_dir, dir_file_bytes, encrypt_bytes, new_dir_id, DIR_FILE,
    DIR_ID_BACKUP_FILE, INFLATED_NAME_FILE,
};
use crate::vault::core::path_map::{child, components};
use crate::vfs::{Backend, VfsMeta};

use super::backend::VaultBackend;
use super::backend_nodes::{invalid_input, not_a_folder, Found, NodeKind, NodeRef, Parts};

/// New directory IDs tried before giving up (a collision of 122 random
/// bits does not happen; the bound only keeps the loop finite).
const DIR_ID_ATTEMPTS: usize = 3;

fn already_exists() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Eintrag im Tresor existiert bereits",
    )
}

impl VaultBackend {
    /// Metadata of a cleartext path.
    pub(super) fn stat_path(&self, path: &str) -> io::Result<VfsMeta> {
        self.session.check()?;
        let result = match Parts::of(path)? {
            None => self.root_meta(),
            Some(parts) => match self.stage_meta(&parts)? {
                Some(meta) => Ok(meta),
                None => self
                    .find_at(&parts)
                    .and_then(|found| self.cleartext_meta(parts.name, &found)),
            },
        };
        result.map_err(|error| self.session.relock(error))
    }

    fn root_meta(&self) -> io::Result<VfsMeta> {
        let root = self.root_dir()?;
        let stored = self.inner.stat(&self.at(&root.content))?;
        if !stored.is_dir || stored.is_symlink || stored.special { return Err(not_a_folder()); }
        Ok(VfsMeta {
            is_dir: true,
            mtime_ms: stored.mtime_ms,
            btime_ms: stored.btime_ms,
            id: stored.id,
            ..VfsMeta::default()
        })
    }

    /// The cleartext metadata of a found entry named `name`.
    pub(super) fn cleartext_meta(&self, name: &str, found: &Found) -> io::Result<VfsMeta> {
        let combo = self.session.config().combo;
        let base = VfsMeta {
            name: name.to_string(),
            hidden: name.starts_with('.'),
            ..VfsMeta::default()
        };
        Ok(match &found.kind {
            NodeKind::File { contents } => VfsMeta {
                size: cleartext_file_size(contents.size, combo)?,
                mtime_ms: contents.mtime_ms,
                btime_ms: contents.btime_ms,
                id: contents.id.clone(),
                ..base
            },
            NodeKind::Dir { node } => VfsMeta {
                is_dir: true,
                mtime_ms: node.mtime_ms,
                btime_ms: node.btime_ms,
                id: node.id.clone(),
                ..base
            },
            NodeKind::Symlink { link, node } => VfsMeta {
                is_symlink: true,
                size: cleartext_file_size(link.size, combo).unwrap_or(0),
                mtime_ms: node.mtime_ms,
                btime_ms: node.btime_ms,
                id: node.id.clone(),
                ..base
            },
        })
    }

    /// The ID of the stored object behind a cleartext path.
    pub(super) fn storage_item_id(&self, path: &str) -> io::Result<Option<String>> {
        self.session.check()?;
        let stored = match Parts::of(path)? {
            None => self.root_dir()?.content,
            Some(parts) => {
                let found = self.find_at(&parts)?;
                match found.kind {
                    NodeKind::File { .. } => found.node.contents(),
                    NodeKind::Dir { .. } | NodeKind::Symlink { .. } => found.node.node(),
                }
            }
        };
        self.inner.item_id(&self.at(&stored))
    }

    /// A strict logical reservation never uses a legacy probe/create default.
    pub(super) fn make_dir_reserved(&self, path: &str) -> io::Result<()> {
        self.session.check()?;
        if !self.inner.staged_write_capabilities(&self.root).create
            || !crate::vfs::directory_reservation_supported(self.inner.as_ref(), &self.root) {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "storage cannot reserve vault metadata exclusively"));
        }
        let parts = Parts::of(path)?.ok_or_else(already_exists)?;
        let parent = self.dir_for(&parts.parent)?;
        let node = self.canonical(&parent, parts.name)?;
        match self.find_at(&parts) {
            Ok(_) => return Err(already_exists()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        let created = self.create_folder(&node, true);
        self.cache.forget_aliases(&parts.parent_key());
        created.map_err(|error| self.session.relock(error))
    }

    /// `create_dir` (`exclusive = false`: an existing folder is success) and
    /// `create_dir_new` (`exclusive = true`: `AlreadyExists`).
    pub(super) fn make_dir(&self, path: &str, exclusive: bool) -> io::Result<()> {
        self.session.check()?;
        let result = match Parts::of(path)? {
            None if exclusive => Err(already_exists()),
            None => Ok(()),
            Some(parts) => self.make_dir_at(&parts, exclusive),
        };
        result.map_err(|error| self.session.relock(error))
    }

    /// Every missing folder of `path`, from the top.
    pub(super) fn make_dirs(&self, path: &str) -> io::Result<()> {
        self.session.check()?;
        let all = components(path)?;
        for end in 1..=all.len() {
            let parts = Parts {
                parent: all[..end - 1].to_vec(),
                name: all[end - 1],
            };
            self.make_dir_at(&parts, false)
                .map_err(|error| self.session.relock(error))?;
        }
        Ok(())
    }

    fn make_dir_at(&self, parts: &Parts<'_>, exclusive: bool) -> io::Result<()> {
        let parent = self.dir_for(&parts.parent)?;
        let parent_key = parts.parent_key();
        match self.find(&parent_key, &parent, parts.name) {
            Ok(found) if !exclusive && matches!(found.kind, NodeKind::Dir { .. }) => return Ok(()),
            Ok(_) => return Err(already_exists()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let node = self.canonical(&parent, parts.name)?;
        let created = self.create_folder(&node, false);
        self.cache.forget_aliases(&parent_key);
        match created {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && !exclusive => {
                // Another client was faster; fine when it made a folder.
                match self.find(&parent_key, &parent, parts.name)?.kind {
                    NodeKind::Dir { .. } => Ok(()),
                    _ => Err(error),
                }
            }
            other => other,
        }
    }

    /// Content directory and its `dirid.c9r` first, then the node (created
    /// exclusively: the no-replace gate), its `name.c9s` and `dir.c9r`.
    fn create_folder(&self, node: &NodeRef, strict: bool) -> io::Result<()> {
        let (id, content) = self.new_content_dir(strict)?;
        let node_path = self.at(&node.node());
        let reserve = if strict { crate::vfs::create_dir_exclusive(&*self.inner, &node_path) } else { self.inner.create_dir_new(&node_path) };
        let created = reserve.and_then(|()| {
            let typed = (|| -> io::Result<()> {
                if node.shortened {
                    self.write_new_file(
                        &node.inside(INFLATED_NAME_FILE),
                        node.encrypted.as_bytes(),
                    )?;
                }
                self.write_new_file(&node.inside(DIR_FILE), &dir_file_bytes(&id))
            })();
            if typed.is_err() {
                self.remove_quietly(
                    &[node.inside(DIR_FILE), node.inside(INFLATED_NAME_FILE)],
                    &node.node(),
                );
            }
            typed
        });
        if created.is_err() {
            self.remove_quietly(&[child(&content, DIR_ID_BACKUP_FILE)], &content);
        }
        created
    }

    /// A new directory ID with its empty content directory and the
    /// encrypted backup of the ID (`dirid.c9r`).
    fn new_content_dir(&self, strict: bool) -> io::Result<(String, String)> {
        for _ in 0..DIR_ID_ATTEMPTS {
            let id = new_dir_id()?;
            let content = self.session.with_key(|key| content_dir(key, &id))?;
            let bucket = content
                .rsplit_once('/')
                .map_or(content.as_str(), |(bucket, _)| bucket);
            match self.inner.create_dir(&self.at(bucket)) {
                Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
                _ => {}
            }
            let create = if strict { crate::vfs::create_dir_exclusive(&*self.inner, &self.at(&content)) } else { self.inner.create_dir_new(&self.at(&content)) };
            match create {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
                Ok(()) => {}
            }
            let backup = encrypt_bytes(&self.session.cryptor(), id.as_bytes())
                .map_err(|error| self.session.format_error(error))?;
            let backup_path = child(&content, DIR_ID_BACKUP_FILE);
            if let Err(error) = self.write_new_file(&backup_path, &backup) {
                self.remove_quietly(&[backup_path], &content);
                return Err(error);
            }
            return Ok((id, content));
        }
        Err(already_exists())
    }

    /// Removes an empty folder: refused while its content directory holds
    /// anything but `dirid.c9r` (also stages or foreign files nobody sees in
    /// the listing), and for the display name of a conflict copy (its
    /// content may be another folder's; „Tresor prüfen“ separates them).
    pub(super) fn remove_folder(&self, path: &str) -> io::Result<()> {
        self.session.check()?;
        let parts = Parts::of(path)?
            .ok_or_else(|| invalid_input("Die Tresorwurzel lässt sich nicht entfernen"))?;
        let result = (|| -> io::Result<_> {
            let found = self.find_at(&parts)?;
            if !matches!(found.kind, NodeKind::Dir { .. }) {
                return Err(not_a_folder());
            }
            if found.node.alias {
                return Err(invalid_input(
                    "Konfliktkopie eines Ordners: über „Tresor prüfen“ bereinigen",
                ));
            }
            let dir = self.read_dir_id(&found.node)?;
            let stored = self.inner.list_dir(&self.at(&dir.content))?;
            if stored.iter().any(|entry| entry.name != DIR_ID_BACKUP_FILE) {
                return Err(io::Error::new(
                    io::ErrorKind::DirectoryNotEmpty,
                    "Ordner im Tresor ist nicht leer",
                ));
            }
            self.remove_node_folder(&found.node, DIR_FILE)?;
            self.cache.forget_tree(&parts.key());
            self.cache.forget_aliases(&parts.parent_key());
            // The folder is gone for every client now; a content directory
            // left by a failure here is an orphan „Tresor prüfen“ reports.
            self.remove_quietly(&[child(&dir.content, DIR_ID_BACKUP_FILE)], &dir.content);
            Ok(())
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// Removes a node folder holding only `type_file` (and `name.c9s` when
    /// shortened); anything else in it refuses before a change.
    pub(super) fn remove_node_folder(&self, node: &NodeRef, type_file: &str) -> io::Result<()> {
        let inside = self.inner.list_dir(&self.at(&node.node()))?;
        let expected =
            |name: &str| name == type_file || (node.shortened && name == INFLATED_NAME_FILE);
        if inside.iter().any(|entry| !expected(&entry.name)) {
            return Err(io::Error::new(
                io::ErrorKind::DirectoryNotEmpty,
                "Eintrag im Tresor enthält fremde Dateien („Tresor prüfen“)",
            ));
        }
        // The type file first: the entry is gone for every reader at once.
        let mut names: Vec<&str> = inside.iter().map(|entry| entry.name.as_str()).collect();
        names.sort_by_key(|name| *name != type_file);
        for name in names {
            self.inner.remove_file(&self.at(&node.inside(name)))?;
        }
        self.inner.remove_dir(&self.at(&node.node()))
    }

    /// Best-effort removal of storage files, then of their folder `dir`.
    pub(super) fn remove_quietly(&self, files: &[String], dir: &str) {
        for file in files {
            let _ = self.inner.remove_file(&self.at(file));
        }
        let _ = self.inner.remove_dir(&self.at(dir));
    }
}
