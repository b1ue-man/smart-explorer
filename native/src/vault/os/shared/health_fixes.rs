//! The fixes of „Tresor prüfen“ (vault plan C6.2, U26-7). Each fix works on
//! the cipher files of one side only, under the pair locks of the vault jobs
//! that use the site. Before any fix removes cipher files, a copy of them
//! goes to `<app data>/vault-removed/<site>/<time>/` and the fix stops when
//! that copy fails; the surfaces ask the user before they call a removal.
//! What a fix makes new is made first, what it removes comes last.
use std::collections::HashSet;
use std::io;
use std::sync::atomic::AtomicBool;

use crate::cryptomator::{
    content_dir, decrypt_bytes, dir_file_bytes, encrypt_bytes, new_dir_id, read_bounded,
    shortened_name, CIPHER_SUFFIX, CONTENTS_FILE, DIR_FILE, DIR_ID_BACKUP_FILE, INFLATED_NAME_FILE,
    INFLATED_NAME_READ_LIMIT, ROOT_DIR_ID, SYMLINK_FILE,
};
use crate::vault::core::compare_model::ItemAction;
use crate::vault::core::health_model::{FindingKind, HealthFinding};
use crate::vault::core::listing::{classify_entry, EntryClass};
use crate::vault::core::path_map::{child, storage_path};
use crate::vault::core::VaultSession;

use super::compare::invalid;
use super::compare_actions::{pair_locks, ActionEnv};
use super::compare_cache::{stale, CompareState, Payload, SideIo};
use super::compare_files::{backup_dir, backup_tree, put_new, remove_tree, replace_bytes};
use super::compare_folders::{ensure_content_dir, leaf_of, publish_as, read_dir_id, source_of};
use super::compare_keep_both::{entry_taken, new_entry};
use super::dir_map::DirMap;

/// The recovery folder of `OrphanContentDir` (C6.2) and its directory ID.
const LOST_FOUND: &str = "LOST+FOUND";
const LOST_FOUND_ID: &str = "recovery";
/// Longest cleartext directory ID `dirid.c9r` holds (36 characters, UUID).
const DIR_ID_LIMIT: usize = 256;

fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn last_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Copies `rel` to the removal folder, then removes it.
fn remove_with_backup(side: &SideIo, rel: &str) -> io::Result<()> {
    let inner = side.backend.as_ref();
    backup_tree(inner, &side.root, rel, &backup_dir(&side.site))?;
    remove_tree(inner, &storage_path(&side.root, rel))
}

/// The directory ID `dirid.c9r` of the content folder `content` names.
fn read_backup_id(side: &SideIo, session: &VaultSession, content: &str) -> io::Result<String> {
    let path = storage_path(&side.root, &child(content, DIR_ID_BACKUP_FILE));
    let bytes = read_bounded(side.backend.open_read(&path)?, DIR_ID_LIMIT * 64)?
        .ok_or_else(|| invalid("dirid.c9r ist zu groß"))?;
    let clear = decrypt_bytes(&session.cryptor(), &bytes, DIR_ID_LIMIT)
        .map_err(|error| session.format_error(error))?;
    String::from_utf8(clear.to_vec()).map_err(|_| invalid("dirid.c9r enthält keine Verzeichnis-ID"))
}

/// Makes the node `leaf` in the content folder `parent` (directory ID
/// `parent_id`) pointing to the folder `child_id`.
fn make_node(
    side: &SideIo,
    session: &VaultSession,
    parent: (&str, &str),
    leaf: &str,
    child_id: &str,
) -> io::Result<()> {
    let entry = new_entry(session, parent.1, leaf)?;
    let inner = side.backend.as_ref();
    let node = storage_path(&side.root, &entry.node(parent.0));
    inner.create_dir_new(&node)?;
    let made = (|| -> io::Result<()> {
        if entry.shortened {
            put_new(
                inner,
                &child(&node, INFLATED_NAME_FILE),
                entry.encrypted.as_bytes(),
            )?;
        }
        put_new(inner, &child(&node, DIR_FILE), &dir_file_bytes(child_id))
    })();
    if made.is_err() {
        let _ = remove_tree(inner, &node);
    }
    made
}

/// `DirIdCollision` that is a plain duplicate: the second node goes.
fn remove_duplicate(side: &SideIo, finding: &HealthFinding) -> io::Result<()> {
    remove_with_backup(side, parent_of(&finding.rel))
}

/// `DirIdCollision` of two folders: the content of the shared folder is
/// copied (names anew, subfolders with new IDs) into a new folder next to
/// the second node, then the second node goes.
fn separate_folder(
    side: &SideIo,
    session: &VaultSession,
    map: &DirMap,
    finding: &HealthFinding,
) -> io::Result<()> {
    let node = parent_of(&finding.rel);
    let (content, stored) = node
        .rsplit_once('/')
        .ok_or_else(|| invalid("Der Ordnerknoten ist unbekannt"))?;
    let leaf = map
        .entry(content, stored)
        .map(|info| last_of(&info.clear).to_string())
        .ok_or_else(|| invalid("Der Name des Ordners ist unbekannt"))?;
    let parent_id = map
        .dirs
        .get(content)
        .map(|dir| dir.id.clone())
        .ok_or_else(|| invalid("Der Elternordner ist unbekannt"))?;
    let shared_id = read_dir_id(side, &finding.rel)?;
    let shared = session.with_key(|key| content_dir(key, &shared_id))?;
    let new_id = new_dir_id().map_err(|error| session.format_error(error))?;
    let target = ensure_content_dir(side, session, &new_id, true)?;
    let mut visited = HashSet::from([shared_id.clone()]);
    copy_deep(
        side,
        session,
        (&shared, &shared_id),
        (&target, &new_id),
        &mut visited,
    )?;
    let name = format!("{leaf} (getrennt)");
    make_node(side, session, (content, &parent_id), &name, &new_id)?;
    remove_with_backup(side, node)
}

/// Copies the entries of the folder `from` into the new folder `to`;
/// subfolders get new IDs, so no ID is shared afterwards. A folder that
/// contains itself through its IDs is copied once.
fn copy_deep(
    side: &SideIo,
    session: &VaultSession,
    from: (&str, &str),
    to: (&str, &str),
    visited: &mut HashSet<String>,
) -> io::Result<()> {
    for meta in side.backend.list_dir(&storage_path(&side.root, from.0))? {
        let skip = meta.is_symlink
            || meta.special
            || meta.name == DIR_ID_BACKUP_FILE
            || meta.name == DIR_FILE
            || crate::vfs::is_staging_name(&meta.name)
            || classify_entry(&meta.name) == EntryClass::Ignored;
        if skip {
            continue;
        }
        let Some(leaf) = leaf_of(session, side, from.0, &meta.name, from.1)? else {
            continue;
        };
        let source = source_of(side, from.0, &meta)?;
        let node_file = source
            .files
            .iter()
            .find(|(name, _)| name == DIR_FILE)
            .map(|(name, _)| child(&child(from.0, &meta.name), name));
        let Some(node_file) = node_file else {
            let entry = new_entry(session, to.1, &leaf)?;
            publish_as(side, to.0, &entry, &source)?;
            continue;
        };
        let old_id = read_dir_id(side, &node_file)?;
        if !visited.insert(old_id.clone()) {
            continue;
        }
        let new_id = new_dir_id().map_err(|error| session.format_error(error))?;
        let new_content = ensure_content_dir(side, session, &new_id, true)?;
        make_node(side, session, to, &leaf, &new_id)?;
        let old_content = session.with_key(|key| content_dir(key, &old_id))?;
        if side
            .backend
            .try_exists(&storage_path(&side.root, &old_content))?
        {
            copy_deep(
                side,
                session,
                (&old_content, &old_id),
                (&new_content, &new_id),
                visited,
            )?;
        }
    }
    Ok(())
}

/// `EmptyDirFile`, `ObeseDirFile`, `LooseDirFile`, `UnknownType`: the
/// broken node (or the loose file) goes. An untyped node that got its type
/// file meanwhile (a sync finished it) stays.
fn remove_node(side: &SideIo, finding: &HealthFinding) -> io::Result<()> {
    let target = match finding.kind {
        FindingKind::EmptyDirFile | FindingKind::ObeseDirFile => parent_of(&finding.rel),
        _ => finding.rel.as_str(),
    };
    if finding.kind == FindingKind::UnknownType {
        let inside = side.backend.list_dir(&storage_path(&side.root, target))?;
        let typed = inside
            .iter()
            .any(|entry| [DIR_FILE, SYMLINK_FILE, CONTENTS_FILE].contains(&entry.name.as_str()));
        if typed {
            return Err(invalid("Der Knoten ist inzwischen vollständig"));
        }
    }
    remove_with_backup(side, target)
}

/// `MissingContentDir`: an empty content folder with its `dirid.c9r`.
fn create_content(
    side: &SideIo,
    session: &VaultSession,
    finding: &HealthFinding,
) -> io::Result<()> {
    let id = read_dir_id(side, &finding.rel)?;
    ensure_content_dir(side, session, &id, true)?;
    Ok(())
}

/// `MissingDirIdBackup`: `dirid.c9r` with the ID the folder has.
fn write_dir_id(
    side: &SideIo,
    session: &VaultSession,
    map: &DirMap,
    finding: &HealthFinding,
) -> io::Result<()> {
    let content = parent_of(&finding.rel);
    let id = map
        .dirs
        .get(content)
        .map(|dir| dir.id.clone())
        .ok_or_else(|| invalid("Der Ordner ist unbekannt"))?;
    let bytes = encrypt_bytes(&session.cryptor(), id.as_bytes())
        .map_err(|error| session.format_error(error))?;
    put_new(
        side.backend.as_ref(),
        &storage_path(&side.root, &finding.rel),
        &bytes,
    )
}

/// `OrphanContentDir`: a node „Wiederherstellt <ID>“ below `LOST+FOUND` in
/// the root points to the folder; no entry moves.
fn attach_orphan(side: &SideIo, session: &VaultSession, finding: &HealthFinding) -> io::Result<()> {
    let orphan = finding.rel.as_str();
    let id = read_backup_id(side, session, orphan)?;
    if session.with_key(|key| content_dir(key, &id))? != orphan {
        return Err(invalid(
            "Die Verzeichnis-ID in dirid.c9r passt nicht zu diesem Inhaltsverzeichnis",
        ));
    }
    let root_content = session.with_key(|key| content_dir(key, ROOT_DIR_ID))?;
    let found = new_entry(session, ROOT_DIR_ID, LOST_FOUND)?;
    let found_node = storage_path(&side.root, &found.node(&root_content));
    let found_id = if side.backend.try_exists(&found_node)? {
        read_dir_id(side, &child(&found.node(&root_content), DIR_FILE))?
    } else {
        ensure_content_dir(side, session, LOST_FOUND_ID, true)?;
        make_node(
            side,
            session,
            (&root_content, ROOT_DIR_ID),
            LOST_FOUND,
            LOST_FOUND_ID,
        )?;
        LOST_FOUND_ID.to_string()
    };
    let found_content = ensure_content_dir(side, session, &found_id, true)?;
    let short: String = id.chars().take(8).collect();
    make_node(
        side,
        session,
        (&found_content, &found_id),
        &format!("Wiederhergestellt {short}"),
        &id,
    )
}

/// `LongShortNamesMismatch`: the folder gets the name its `name.c9s` gives.
fn rename_shortened(side: &SideIo, finding: &HealthFinding) -> io::Result<()> {
    let right = finding
        .detail
        .as_deref()
        .ok_or_else(|| invalid("Der richtige Kurzname ist unbekannt"))?;
    let target = child(parent_of(&finding.rel), right);
    let inner = side.backend.as_ref();
    let to = storage_path(&side.root, &target);
    if inner.try_exists(&to)? {
        return Err(invalid("Der Name ist belegt"));
    }
    inner.rename(&storage_path(&side.root, &finding.rel), &to)
}

/// `TrailingBytesInNameFile`: `name.c9s` holds only the valid name.
fn rewrite_name_file(side: &SideIo, finding: &HealthFinding) -> io::Result<()> {
    let stored = finding.rel.rsplit('/').next().unwrap_or_default();
    let file = child(&finding.rel, INFLATED_NAME_FILE);
    let path = storage_path(&side.root, &file);
    let bytes = read_bounded(side.backend.open_read(&path)?, INFLATED_NAME_READ_LIMIT)?
        .ok_or_else(|| invalid("name.c9s ist zu groß"))?;
    let text = String::from_utf8(bytes).map_err(|_| invalid("name.c9s ist kein Text"))?;
    let end = text
        .find(CIPHER_SUFFIX)
        .map(|at| at + CIPHER_SUFFIX.len())
        .filter(|end| shortened_name(&text[..*end]) == stored)
        .ok_or_else(|| invalid("name.c9s enthält keinen gültigen Namen"))?;
    backup_tree(
        side.backend.as_ref(),
        &side.root,
        &file,
        &backup_dir(&side.site),
    )?;
    replace_bytes(side.backend.as_ref(), &path, text[..end].as_bytes())
}

/// `ForeignConflictCopy`: the entry gets the canonical name of its cleartext.
fn canonical_name(
    side: &SideIo,
    session: &VaultSession,
    map: &DirMap,
    finding: &HealthFinding,
) -> io::Result<()> {
    let (content, stored) = finding
        .rel
        .rsplit_once('/')
        .ok_or_else(|| invalid("Der Eintrag ist unbekannt"))?;
    let info = map
        .entry(content, stored)
        .ok_or_else(|| invalid("Der Eintrag ist unbekannt"))?;
    let parent_id = map
        .dirs
        .get(content)
        .map(|dir| dir.id.clone())
        .ok_or_else(|| invalid("Der Ordner des Eintrags ist unbekannt"))?;
    let entry = new_entry(session, &parent_id, last_of(&info.clear))?;
    if entry_taken(std::slice::from_ref(side), content, &entry.stored) {
        return Err(invalid("Name belegt"));
    }
    let mut meta = side.backend.stat(&storage_path(&side.root, &finding.rel))?;
    meta.name = stored.to_string();
    let source = source_of(side, content, &meta)?;
    publish_as(side, content, &entry, &source)?;
    remove_with_backup(side, &finding.rel)
}

/// Runs one fix of „Tresor prüfen“ on the entry `index`.
pub(super) fn fix(
    env: &ActionEnv<'_>,
    state: &CompareState,
    index: usize,
    action: ItemAction,
    _cancel: &AtomicBool,
) -> io::Result<()> {
    let (side_index, finding) = match state.items().get(index).map(|held| &held.payload) {
        Some(Payload::Finding { side, finding }) => (*side, finding.clone()),
        Some(Payload::Files { .. }) => {
            return Err(invalid("Dieser Eintrag ist kein Befund der Prüfung"))
        }
        None => return Err(stale()),
    };
    let side = state.sides.get(side_index).ok_or_else(stale)?;
    let map = state.maps.get(side_index).ok_or_else(stale)?;
    let session: &VaultSession = &state.session;
    let _locks = pair_locks(env, state)?;
    match action {
        ItemAction::RemoveDuplicate => remove_duplicate(side, &finding),
        ItemAction::SeparateFolder => separate_folder(side, session, map, &finding),
        ItemAction::RemoveNode => remove_node(side, &finding),
        ItemAction::CreateContent => create_content(side, session, &finding),
        ItemAction::WriteDirId => write_dir_id(side, session, map, &finding),
        ItemAction::AttachOrphan => attach_orphan(side, session, &finding),
        ItemAction::RenameShortened => rename_shortened(side, &finding),
        ItemAction::RewriteNameFile => rewrite_name_file(side, &finding),
        ItemAction::CanonicalName => canonical_name(side, session, map, &finding),
        ItemAction::RemoveStage => remove_with_backup(side, &finding.rel),
        ItemAction::KeepVersionA | ItemAction::KeepVersionB => Err(invalid(
            "Schlüsseldateien werden im Vergleich eines Tresor-Jobs gewählt",
        )),
        _ => Err(invalid("Diese Aktion gehört nicht zur Prüfung")),
    }
}
