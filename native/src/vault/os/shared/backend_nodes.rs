//! How a cleartext path is found in the vault's storage: folders by their
//! directory ID (read from `dir.c9r`, then cached), entries by their
//! encrypted name, foreign conflict copies by the display name the listing
//! gave them (C1.8). Links inside the vault are never followed: a path
//! through a link ends with `NotADirectory`.
use std::io;
use std::sync::Arc;

use crate::cryptomator::{
    content_dir, encrypt_name, needs_shortening, parse_dir_file, read_bounded, shortened_name,
    CONTENTS_FILE, DIR_FILE, DIR_FILE_READ_LIMIT, ROOT_DIR_ID, SYMLINK_FILE,
};
use crate::vault::core::listing::{
    node_shape, AliasMap, NodeShape, AMBIGUOUS_NODE, STORAGE_LINK, UNTYPED_NODE,
};
use crate::vault::core::path_map::{child, components, dir_key, stage_plan};
use crate::vfs::{Backend, VfsMeta};

use super::backend::VaultBackend;
use super::dir_cache::DirRef;

/// One entry of a content directory as stored.
#[derive(Clone, Debug)]
pub(super) struct NodeRef {
    pub(super) parent: DirRef,
    /// The stored name (`<base64>.c9r`, `<digest>.c9s`, or the name of a
    /// foreign conflict copy).
    pub(super) stored: String,
    /// The full encrypted name the entry decrypts with.
    pub(super) encrypted: String,
    pub(super) shortened: bool,
    /// Reached through the display name of a foreign conflict copy.
    pub(super) alias: bool,
}

impl NodeRef {
    /// The entry itself, relative to the vault root.
    pub(super) fn node(&self) -> String {
        child(&self.parent.content, &self.stored)
    }

    /// A file inside a node folder.
    pub(super) fn inside(&self, name: &str) -> String {
        child(&self.node(), name)
    }

    /// The encrypted content of a file entry.
    pub(super) fn contents(&self) -> String {
        if self.shortened {
            self.inside(CONTENTS_FILE)
        } else {
            self.node()
        }
    }
}

/// What a found entry is, with the storage metadata the cleartext metadata
/// comes from.
#[derive(Clone, Debug)]
pub(super) enum NodeKind {
    File { contents: VfsMeta },
    Dir { node: VfsMeta },
    Symlink { link: VfsMeta, node: VfsMeta },
}

#[derive(Clone, Debug)]
pub(super) struct Found {
    pub(super) node: NodeRef,
    pub(super) kind: NodeKind,
}

pub(super) fn not_found() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "Eintrag im Tresor nicht gefunden")
}

pub(super) fn not_a_folder() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotADirectory,
        "Eintrag im Tresor ist kein Ordner (Verknüpfungen werden nicht verfolgt)",
    )
}

pub(super) fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

pub(super) fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

/// One literal name component: never empty, `.`, `..`, or with `/`/NUL.
pub(super) fn check_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        Err(invalid_input("Ungültiger Name im Tresor"))
    } else {
        Ok(())
    }
}

/// A cleartext path split into its parent folder and its last name.
pub(super) struct Parts<'a> {
    pub(super) parent: Vec<&'a str>,
    pub(super) name: &'a str,
}

impl<'a> Parts<'a> {
    /// `None` for the vault root.
    pub(super) fn of(path: &'a str) -> io::Result<Option<Self>> {
        let mut all = components(path)?;
        Ok(all.pop().map(|name| Self { parent: all, name }))
    }

    /// The cache key of the parent folder.
    pub(super) fn parent_key(&self) -> String {
        dir_key(&self.parent)
    }

    /// The cache key of the entry as a folder.
    pub(super) fn key(&self) -> String {
        let mut all = self.parent.clone();
        all.push(self.name);
        dir_key(&all)
    }
}

impl VaultBackend {
    /// The vault root: directory ID `""`.
    pub(super) fn root_dir(&self) -> io::Result<DirRef> {
        let content = self.session.with_key(|key| content_dir(key, ROOT_DIR_ID))?;
        Ok(DirRef {
            id: ROOT_DIR_ID.to_string(),
            content,
        })
    }

    /// The folder of `components` (cached for 20 s).
    pub(super) fn dir_for(&self, components: &[&str]) -> io::Result<DirRef> {
        let key = dir_key(components);
        if let Some(dir) = self.cache.dir(&key) {
            return Ok(dir);
        }
        let dir = match components.split_last() {
            None => self.root_dir()?,
            Some((name, parent)) => {
                let parent_dir = self.dir_for(parent)?;
                self.child_dir(&dir_key(parent), &parent_dir, name)?
            }
        };
        self.cache.put_dir(&key, &dir);
        Ok(dir)
    }

    /// The folder `name` in `parent`: one read of `dir.c9r` of its
    /// canonical entry; only when that fails, a closer look for the reason
    /// (a file, a link, a conflict copy, nothing).
    fn child_dir(&self, parent_key: &str, parent: &DirRef, name: &str) -> io::Result<DirRef> {
        let node = self.canonical(parent, name)?;
        match self.read_dir_id(&node) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => return result,
        }
        let found = self.find(parent_key, parent, name)?;
        match found.kind {
            NodeKind::Dir { .. } => self.read_dir_id(&found.node),
            NodeKind::File { .. } | NodeKind::Symlink { .. } => Err(not_a_folder()),
        }
    }

    /// The folder a folder entry stands for: its `dir.c9r`.
    pub(super) fn read_dir_id(&self, node: &NodeRef) -> io::Result<DirRef> {
        let reader = self.inner.open_read(&self.at(&node.inside(DIR_FILE)))?;
        let bytes = read_bounded(reader, DIR_FILE_READ_LIMIT)?
            .ok_or_else(|| invalid_data("dir.c9r ist zu groß"))?;
        let id = parse_dir_file(&bytes)?;
        let content = self.session.with_key(|key| content_dir(key, &id))?;
        Ok(DirRef { id, content })
    }

    /// The canonical stored entry of `name` in `parent` (no storage access):
    /// `<base64>.c9r`, or `<digest>.c9s` above the shortening threshold.
    pub(super) fn canonical(&self, parent: &DirRef, name: &str) -> io::Result<NodeRef> {
        check_name(name)?;
        let encrypted = self
            .session
            .with_key(|key| encrypt_name(key, name, &parent.id))?;
        let shortened = needs_shortening(&encrypted, self.session.config().shortening_threshold);
        let stored = if shortened {
            shortened_name(&encrypted)
        } else {
            encrypted.clone()
        };
        Ok(NodeRef {
            parent: parent.clone(),
            stored,
            encrypted,
            shortened,
            alias: false,
        })
    }

    /// The entry `name` in `parent`: the canonical one, else a foreign
    /// conflict copy listed under this display name. `NotFound` when none.
    pub(super) fn find(&self, parent_key: &str, parent: &DirRef, name: &str) -> io::Result<Found> {
        match self.examine(self.canonical(parent, name)?) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => return result,
        }
        let aliases = self.aliases(parent_key, parent)?;
        let target = aliases.get(name).ok_or_else(not_found)?;
        self.examine(NodeRef {
            parent: parent.clone(),
            stored: target.stored.clone(),
            encrypted: target.encrypted.clone(),
            shortened: target.shortened,
            alias: true,
        })
    }

    /// `find` at a cleartext path split into parts. A stage-like name is
    /// only looked up as written (no conflict-copy display names): probing
    /// a stage name never lists its folder.
    pub(super) fn find_at(&self, parts: &Parts<'_>) -> io::Result<Found> {
        let parent = self.dir_for(&parts.parent)?;
        if stage_plan(parts.name).is_some() {
            return self.examine(self.canonical(&parent, parts.name)?);
        }
        self.find(&parts.parent_key(), &parent, parts.name)
    }

    /// The display names of conflict copies in `parent` (listed when not
    /// cached).
    pub(super) fn aliases(&self, parent_key: &str, parent: &DirRef) -> io::Result<Arc<AliasMap>> {
        match self.cache.aliases(parent_key) {
            Some(aliases) => Ok(aliases),
            None => {
                let scan = self.scan(parent)?;
                Ok(self.cache.put_aliases(parent_key, scan.aliases))
            }
        }
    }

    /// The kind of a stored entry; `NotFound` when it does not exist.
    pub(super) fn examine(&self, node: NodeRef) -> io::Result<Found> {
        let meta = self.inner.stat(&self.at(&node.node()))?;
        if meta.is_symlink || meta.special {
            return Err(invalid_data(STORAGE_LINK));
        }
        if !meta.is_dir {
            if node.shortened {
                return Err(invalid_data(UNTYPED_NODE));
            }
            return Ok(Found {
                node,
                kind: NodeKind::File { contents: meta },
            });
        }
        let children = self.inner.list_dir(&self.at(&node.node()))?;
        Self::classify(node, meta, &children)
    }

    /// The kind of a node folder, from the type files listed inside.
    pub(super) fn classify(
        node: NodeRef,
        meta: VfsMeta,
        children: &[VfsMeta],
    ) -> io::Result<Found> {
        let file = |name: &str| {
            children
                .iter()
                .find(|entry| entry.name == name && !entry.is_dir && !entry.is_symlink)
                .cloned()
        };
        let (dir_file, link_file, contents) =
            (file(DIR_FILE), file(SYMLINK_FILE), file(CONTENTS_FILE));
        let shape = node_shape(
            node.shortened,
            dir_file.is_some(),
            link_file.is_some(),
            contents.is_some(),
        );
        let kind = match (shape, link_file, contents) {
            (NodeShape::Dir, _, _) => NodeKind::Dir { node: meta },
            (NodeShape::Symlink, Some(link), _) => NodeKind::Symlink { link, node: meta },
            (NodeShape::File, _, Some(contents)) => NodeKind::File { contents },
            (NodeShape::Ambiguous, _, _) => return Err(invalid_data(AMBIGUOUS_NODE)),
            _ => return Err(invalid_data(UNTYPED_NODE)),
        };
        Ok(Found { node, kind })
    }
}
