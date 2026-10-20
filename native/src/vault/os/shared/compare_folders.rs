//! „Ordner zusammenführen“ and „Beide Ordner behalten“ (vault plan C6.4,
//! R3, U26-6): both sides created a folder of one name, so the node
//! `<name>.c9r/dir.c9r` names two different directory IDs. Everything the
//! actions do is done in the cipher: names are encrypted anew with the
//! directory ID of the target folder, the cipher bytes of the entries stay
//! as they are. New entries are made first and old ones removed last, after
//! a copy of them went to `vault-removed/`, so a crash leaves duplicates
//! („Tresor prüfen“ finds them) and never a loss.
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::bisync::resolve_recorded;
use crate::cryptomator::{
    content_dir, decrypt_name, dir_file_bytes, encrypt_bytes, parse_dir_file, read_bounded,
    CONTENTS_FILE, DIR_FILE, DIR_FILE_READ_LIMIT, DIR_ID_BACKUP_FILE, INFLATED_NAME_FILE,
    INFLATED_NAME_READ_LIMIT, SHORTENED_SUFFIX,
};
use crate::vault::core::conflict_names::{conflict_name, folder_conflict_name};
use crate::vault::core::listing::{classify_entry, EntryClass};
use crate::vault::core::path_map::{child, storage_path};
use crate::vault::core::VaultSession;
use crate::vfs::{Backend, VfsMeta};

use super::compare::invalid;
use super::compare_actions::{files_of, lock_preview, pair, pair_locks, ActionEnv};
use super::compare_cache::{stale, CompareState, SideIo};
use super::compare_files::{backup_dir, backup_tree, copy_new, put_new, remove_tree};
use super::compare_keep_both::{entry_taken, new_entry, NewEntry};

/// At most this many bytes are compared in one step.
const COMPARE_CHUNK: usize = 64 * 1024;

fn read_small(side: &SideIo, rel: &str, limit: usize) -> io::Result<Option<Vec<u8>>> {
    read_bounded(
        side.backend.open_read(&storage_path(&side.root, rel))?,
        limit,
    )
}

/// The directory ID in the node file `node_file` (`…/dir.c9r`).
pub(super) fn read_dir_id(side: &SideIo, node_file: &str) -> io::Result<String> {
    let bytes = read_small(side, node_file, DIR_FILE_READ_LIMIT)?
        .ok_or_else(|| invalid("dir.c9r ist zu groß"))?;
    parse_dir_file(&bytes).map_err(|_| invalid("dir.c9r enthält keine Verzeichnis-ID"))
}

/// The content folder of the directory ID `id`, made when it is missing:
/// with its `dirid.c9r` when `with_backup`, else only the folder (the sync
/// brings the `dirid.c9r` of the other side, so no second one conflicts).
pub(super) fn ensure_content_dir(
    side: &SideIo,
    session: &VaultSession,
    id: &str,
    with_backup: bool,
) -> io::Result<String> {
    let content = session.with_key(|key| content_dir(key, id))?;
    let inner = side.backend.as_ref();
    let path = storage_path(&side.root, &content);
    if inner.try_exists(&path)? {
        return Ok(content);
    }
    let bucket = content
        .rsplit_once('/')
        .map_or(content.as_str(), |(bucket, _)| bucket);
    inner.mkdir_all(&storage_path(&side.root, bucket))?;
    inner.create_dir_new(&path)?;
    if with_backup {
        let backup = encrypt_bytes(&session.cryptor(), id.as_bytes())
            .map_err(|error| session.format_error(error))?;
        put_new(inner, &child(&path, DIR_ID_BACKUP_FILE), &backup)?;
    }
    Ok(content)
}

/// The full encrypted name an entry of `content` stands for: the stored
/// name, or the name inside `name.c9s` of a shortened entry.
fn encrypted_of(side: &SideIo, content: &str, stored: &str) -> io::Result<Option<String>> {
    if !stored.ends_with(SHORTENED_SUFFIX) {
        return Ok(Some(stored.to_string()));
    }
    let rel = child(&child(content, stored), INFLATED_NAME_FILE);
    let Some(bytes) = read_small(side, &rel, INFLATED_NAME_READ_LIMIT)? else {
        return Ok(None);
    };
    Ok(String::from_utf8(bytes).ok())
}

/// The cleartext name of the entry `stored` in a folder with ID `id`.
pub(super) fn leaf_of(
    session: &VaultSession,
    side: &SideIo,
    content: &str,
    stored: &str,
    id: &str,
) -> io::Result<Option<String>> {
    let Some(encrypted) = encrypted_of(side, content, stored)? else {
        return Ok(None);
    };
    let attempt = session.with_key(|key| Ok(decrypt_name(key, &encrypted, id)))?;
    Ok(attempt.ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    /// The bytes of one file.
    File,
    /// A folder or link node: its type files.
    Node,
}

/// The cipher files that make up one entry.
pub(super) struct Source {
    pub(super) shape: Shape,
    /// (name inside the node, storage path); the name is empty for a plain
    /// file.
    pub(super) files: Vec<(String, String)>,
}

pub(super) fn source_of(side: &SideIo, content: &str, meta: &VfsMeta) -> io::Result<Source> {
    let path = storage_path(&side.root, &child(content, &meta.name));
    if !meta.is_dir {
        return Ok(Source {
            shape: Shape::File,
            files: vec![(String::new(), path)],
        });
    }
    let inside = side.backend.list_dir(&path)?;
    if inside
        .iter()
        .any(|entry| entry.name == CONTENTS_FILE && !entry.is_dir)
    {
        return Ok(Source {
            shape: Shape::File,
            files: vec![(CONTENTS_FILE.to_string(), child(&path, CONTENTS_FILE))],
        });
    }
    let files = inside
        .iter()
        .filter(|entry| !entry.is_dir && !entry.is_symlink && entry.name != INFLATED_NAME_FILE)
        .map(|entry| (entry.name.clone(), child(&path, &entry.name)))
        .collect();
    Ok(Source {
        shape: Shape::Node,
        files,
    })
}

/// Publishes `source` as the entry `entry` of the content folder `to`.
pub(super) fn publish_as(
    side: &SideIo,
    to: &str,
    entry: &NewEntry,
    source: &Source,
) -> io::Result<()> {
    let inner = side.backend.as_ref();
    let node = storage_path(&side.root, &entry.node(to));
    if source.shape == Shape::File && !entry.shortened {
        return copy_new(inner, &source.files[0].1, &node);
    }
    inner.create_dir_new(&node)?;
    let made = (|| -> io::Result<()> {
        if entry.shortened {
            put_new(
                inner,
                &child(&node, INFLATED_NAME_FILE),
                entry.encrypted.as_bytes(),
            )?;
        }
        for (name, from) in &source.files {
            let name = if source.shape == Shape::File {
                CONTENTS_FILE
            } else {
                name.as_str()
            };
            copy_new(inner, from, &child(&node, name))?;
        }
        Ok(())
    })();
    if made.is_err() {
        let _ = remove_tree(inner, &node);
    }
    made
}

fn read_full(reader: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = reader.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

fn same_bytes(inner: &dyn Backend, left: &str, right: &str) -> io::Result<bool> {
    if inner.stat(left)?.size != inner.stat(right)?.size {
        return Ok(false);
    }
    let (mut x, mut y) = (inner.open_read(left)?, inner.open_read(right)?);
    let (mut bx, mut by) = (vec![0u8; COMPARE_CHUNK], vec![0u8; COMPARE_CHUNK]);
    loop {
        let nx = read_full(&mut x, &mut bx)?;
        let ny = read_full(&mut y, &mut by)?;
        if nx != ny || bx[..nx] != by[..ny] {
            return Ok(false);
        }
        if nx == 0 {
            return Ok(true);
        }
    }
}

/// Whether `entry` of `to` already holds exactly the bytes of `source` (a
/// repeated action finds its earlier work and does not duplicate it).
fn already_there(side: &SideIo, to: &str, entry: &NewEntry, source: &Source) -> bool {
    let node = storage_path(&side.root, &entry.node(to));
    source.files.iter().all(|(name, from)| {
        let target = match (source.shape, entry.shortened) {
            (Shape::File, false) => node.clone(),
            (Shape::File, true) => child(&node, CONTENTS_FILE),
            (Shape::Node, _) => child(&node, name),
        };
        same_bytes(side.backend.as_ref(), from, &target).unwrap_or(false)
    })
}

/// From where to where `relocate_entries` moves.
pub(super) struct Move<'a> {
    pub(super) from: &'a str,
    pub(super) from_id: &'a str,
    pub(super) to: &'a str,
    pub(super) to_id: &'a str,
    /// The label a conflict name carries.
    pub(super) label: &'a str,
}

/// Copies every entry of the content folder `from` into `to`, encrypting
/// its name with `to_id`; an entry that finds its name taken by other bytes
/// gets a conflict name. Returns the stored names of the entries that are
/// now in `to` (the caller removes those from `from`).
pub(super) fn relocate_entries(
    side: &SideIo,
    session: &VaultSession,
    movement: &Move<'_>,
    stamp: &str,
) -> io::Result<Vec<String>> {
    let sides = std::slice::from_ref(side);
    let mut moved = Vec::new();
    for meta in side
        .backend
        .list_dir(&storage_path(&side.root, movement.from))?
    {
        let skip = meta.is_symlink
            || meta.special
            || meta.name == DIR_ID_BACKUP_FILE
            || meta.name == DIR_FILE
            || crate::vfs::is_staging_name(&meta.name)
            || classify_entry(&meta.name) == EntryClass::Ignored;
        if skip {
            continue;
        }
        let Some(leaf) = leaf_of(session, side, movement.from, &meta.name, movement.from_id)?
        else {
            continue;
        };
        let source = source_of(side, movement.from, &meta)?;
        let mut entry = new_entry(session, movement.to_id, &leaf)?;
        if entry_taken(sides, movement.to, &entry.stored) {
            if already_there(side, movement.to, &entry, &source) {
                moved.push(meta.name.clone());
                continue;
            }
            let taken = |candidate: &str| match new_entry(session, movement.to_id, candidate) {
                Ok(other) => entry_taken(sides, movement.to, &other.stored),
                Err(_) => true,
            };
            let renamed = if source.shape == Shape::Node {
                folder_conflict_name(&leaf, stamp, movement.label, &taken)
            } else {
                conflict_name(&leaf, stamp, movement.label, &taken)
            };
            entry = new_entry(session, movement.to_id, &renamed)?;
        }
        publish_as(side, movement.to, &entry, &source)?;
        moved.push(meta.name.clone());
    }
    Ok(moved)
}

/// Removes the moved entries of the content folder `from` and, when nothing
/// else is left in it, the folder; a copy of the whole folder goes to
/// `backup` first, and without it nothing is removed.
pub(super) fn drop_moved(
    side: &SideIo,
    from: &str,
    moved: &[String],
    backup: &Path,
) -> io::Result<()> {
    let inner = side.backend.as_ref();
    backup_tree(inner, &side.root, from, backup)?;
    for stored in moved {
        remove_tree(inner, &storage_path(&side.root, &child(from, stored)))?;
    }
    let path = storage_path(&side.root, from);
    let rest = inner.list_dir(&path)?;
    if rest.iter().all(|entry| entry.name == DIR_ID_BACKUP_FILE) {
        if !rest.is_empty() {
            inner.remove_file(&child(&path, DIR_ID_BACKUP_FILE))?;
        }
        inner.remove_dir(&path)?;
    }
    Ok(())
}

/// The conflict on a folder node of the entry `index`.
fn node_conflict(state: &CompareState, index: usize) -> io::Result<crate::bisync::Conflict> {
    let (_, conflicts, _) = files_of(state, index)?;
    conflicts
        .into_iter()
        .find(|conflict| {
            conflict.a.is_some() && conflict.b.is_some() && conflict.rel.ends_with("/dir.c9r")
        })
        .ok_or_else(|| invalid("Dieser Eintrag ist kein Konflikt zweier Ordner"))
}

/// „Ordner zusammenführen“: side A's folder (directory ID `idA`) takes the
/// entries of side B's (`idB`); both sides then point to `idA`.
pub(super) fn merge_folders(
    env: &ActionEnv<'_>,
    state: &CompareState,
    index: usize,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (a, b) = pair(state)?;
    let conflict = node_conflict(state, index)?;
    let (id_a, id_b) = (
        read_dir_id(a, &conflict.rel)?,
        read_dir_id(b, &conflict.rel)?,
    );
    if id_a == id_b {
        return Err(invalid("Beide Seiten zeigen schon auf denselben Ordner"));
    }
    let session = &state.session;
    let content_a = session.with_key(|key| content_dir(key, &id_a))?;
    let content_b = session.with_key(|key| content_dir(key, &id_b))?;
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M").to_string();
    let mut absorbed: Vec<(&SideIo, Vec<String>)> = Vec::new();
    {
        let _locks = pair_locks(env, state)?;
        for side in [a, b] {
            if !side
                .backend
                .try_exists(&storage_path(&side.root, &content_b))?
            {
                continue;
            }
            let other_has_backup = state.sides.iter().any(|other| {
                other
                    .backend
                    .try_exists(&storage_path(
                        &other.root,
                        &child(&content_a, DIR_ID_BACKUP_FILE),
                    ))
                    .unwrap_or(false)
            });
            ensure_content_dir(side, session, &id_a, !other_has_backup)?;
            let movement = Move {
                from: &content_b,
                from_id: &id_b,
                to: &content_a,
                to_id: &id_a,
                label: &b.label,
            };
            absorbed.push((side, relocate_entries(side, session, &movement, &stamp)?));
        }
    }
    {
        // Both sides point to the folder of A from here on (the engine
        // copies A's node file over B's and records the result).
        let preview = lock_preview(state)?;
        let key = preview
            .preview
            .state
            .as_ref()
            .ok_or_else(|| invalid("Die Vorschau hat keinen Zustandsbezug"))?;
        resolve_recorded(
            a.backend.as_ref(),
            &a.root,
            b.backend.as_ref(),
            &b.root,
            &conflict,
            true,
            None,
            key,
            cancel,
            |_phase| (),
        )?;
    }
    let _locks = pair_locks(env, state)?;
    for (side, moved) in absorbed {
        drop_moved(side, &content_b, &moved, &backup_dir(&side.site))?;
    }
    Ok(())
}

/// „Beide Ordner behalten“: the node of side B gets a conflict name (and
/// keeps its directory ID and its content), side A's node keeps the name.
pub(super) fn keep_folders_both(
    env: &ActionEnv<'_>,
    state: &CompareState,
    index: usize,
    _cancel: &AtomicBool,
) -> io::Result<()> {
    let (_, b) = pair(state)?;
    let conflict = node_conflict(state, index)?;
    let node_rel = conflict
        .rel
        .strip_suffix("/dir.c9r")
        .ok_or_else(|| invalid("Der Ordnerknoten ist unbekannt"))?;
    let (content, _) = node_rel
        .rsplit_once('/')
        .ok_or_else(|| invalid("Der Ordnerknoten ist unbekannt"))?;
    let id_b = read_dir_id(b, &conflict.rel)?;
    let path = state
        .items()
        .get(index)
        .map(|held| held.item.path.clone())
        .ok_or_else(stale)?;
    let (_, leaf) = path
        .rsplit_once('/')
        .ok_or_else(|| invalid("Der Eintrag hat keinen Pfad"))?;
    let parent_id = state
        .maps
        .iter()
        .find_map(|map| map.dirs.get(content))
        .map(|dir| dir.id.clone())
        .ok_or_else(|| invalid("Der Ordner des Eintrags ist unbekannt"))?;
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M").to_string();
    let session = &state.session;
    let taken = |candidate: &str| match new_entry(session, &parent_id, candidate) {
        Ok(entry) => entry_taken(&state.sides, content, &entry.stored),
        Err(_) => true,
    };
    let renamed = folder_conflict_name(leaf, &stamp, &b.label, &taken);
    let entry = new_entry(session, &parent_id, &renamed)?;
    let _locks = pair_locks(env, state)?;
    let inner = b.backend.as_ref();
    let node = storage_path(&b.root, &entry.node(content));
    inner.create_dir_new(&node)?;
    let made = (|| -> io::Result<()> {
        if entry.shortened {
            put_new(
                inner,
                &child(&node, INFLATED_NAME_FILE),
                entry.encrypted.as_bytes(),
            )?;
        }
        put_new(inner, &child(&node, DIR_FILE), &dir_file_bytes(&id_b))
    })();
    if let Err(error) = made {
        let _ = remove_tree(inner, &node);
        return Err(error);
    }
    backup_tree(inner, &b.root, node_rel, &backup_dir(&b.site))?;
    remove_tree(inner, &storage_path(&b.root, node_rel))
}
