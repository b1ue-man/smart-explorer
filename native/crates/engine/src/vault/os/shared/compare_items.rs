//! The entries of a comparison (vault plan C6.2, U26-4): the actions and
//! conflicts of the engine's preview, keyed by cipher file, become cleartext
//! entries. Several cipher files of one name (`.c9r`, `.c9s` with
//! `name.c9s` and `contents.c9r`, `dirid.c9r`, `dir.c9r`) are one entry; the
//! group of an entry follows its main file, the strongest group wins.
use std::collections::BTreeMap;
use std::io;

use crate::bisync::{Action, Conflict};
use crate::cryptomator::{is_metadata_name, DATA_DIR, DIR_ID_BACKUP_FILE, INFLATED_NAME_FILE};
use crate::syncjobs::SyncJob;
use crate::vault::core::compare_model::{CompareGroup, CompareItem, ItemAction, ItemKind};
use crate::vault::core::listing::{NodeShape, UNDECRYPTABLE};
use crate::vault::core::path_map::storage_path;
use crate::vault::core::sync_rules::action_rel;
use crate::vault::core::VaultSession;

use super::compare::{item_id, side_state, signature_of};
use super::compare_cache::{Held, Payload, SideIo};
use super::compare_merge::mergeable;
use super::dir_map::DirMap;
use super::sync_job::VaultPreview;

/// How a cipher file of the preview belongs to the cleartext world.
pub(super) struct Located {
    /// Entries with the same key are one entry.
    pub(super) key: String,
    pub(super) kind: ItemKind,
    pub(super) path: String,
    pub(super) note: Option<String>,
}

pub(super) fn is_technical(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name == DIR_ID_BACKUP_FILE || name == INFLATED_NAME_FILE
}

/// Maps a cipher file (`rel`, relative to the vault root) to its entry
/// (U26-4): the map of side A decides, else the one of side B.
pub(super) fn locate(rel: &str, maps: &[DirMap]) -> Located {
    let parts: Vec<&str> = rel.trim_start_matches('/').split('/').collect();
    if parts.len() == 1 && is_metadata_name(parts[0]) {
        return Located {
            key: format!("K:{rel}"),
            kind: ItemKind::Key,
            path: String::new(),
            note: None,
        };
    }
    if parts.len() >= 4 && parts[0] == DATA_DIR {
        let content = parts[..3].join("/");
        if parts.len() == 4 && parts[3] == DIR_ID_BACKUP_FILE {
            if let Some(info) = maps.iter().find_map(|map| map.dirs.get(&content)) {
                return Located {
                    key: format!("F:{}", info.clear),
                    kind: ItemKind::Folder,
                    path: info.clear.clone(),
                    note: Some("Verzeichnis-ID-Sicherung".to_string()),
                };
            }
        }
        if let Some(info) = maps
            .iter()
            .find_map(|map| map.entry(&content, parts[3]))
            .filter(|info| parts.len() <= 5)
        {
            let folder = info.shape == NodeShape::Dir;
            return Located {
                key: format!("{}:{}", if folder { 'F' } else { 'E' }, info.clear),
                kind: if folder {
                    ItemKind::Folder
                } else {
                    ItemKind::File
                },
                path: info.clear.clone(),
                note: None,
            };
        }
    }
    Located {
        key: format!("R:{rel}"),
        kind: ItemKind::Node,
        path: String::new(),
        note: Some(UNDECRYPTABLE.to_string()),
    }
}

/// One cipher file of an entry with what the preview says about it.
struct FileEntry {
    rel: String,
    group: CompareGroup,
    action: Option<Action>,
    conflict: Option<Conflict>,
}

struct Acc {
    located: Located,
    files: Vec<FileEntry>,
}

fn missing(side: &SideIo, rel: &str) -> bool {
    matches!(
        side.backend.stat(&storage_path(&side.root, rel)),
        Err(error) if error.kind() == io::ErrorKind::NotFound
    )
}

pub(super) fn group_of(
    action: &Action,
    deferred: &[(Action, String)],
    sides: &[SideIo],
) -> CompareGroup {
    if deferred.iter().any(|(held, _)| held == action) {
        return CompareGroup::Deferred;
    }
    let rel = action_rel(action);
    match action {
        Action::CopyAtoB(_) | Action::KeepBothAtoB(_) if missing(&sides[1], rel) => {
            CompareGroup::NewA
        }
        Action::CopyAtoB(_) | Action::KeepBothAtoB(_) | Action::FinalizeMoveAtoB(_) => {
            CompareGroup::ChangedA
        }
        Action::CopyBtoA(_) | Action::KeepBothBtoA(_) if missing(&sides[0], rel) => {
            CompareGroup::NewB
        }
        Action::CopyBtoA(_) | Action::KeepBothBtoA(_) | Action::FinalizeMoveBtoA(_) => {
            CompareGroup::ChangedB
        }
        Action::DeleteB(_) => CompareGroup::DeletedA,
        Action::DeleteA(_) => CompareGroup::DeletedB,
    }
}

/// The conflicts of the rule stage carry no signatures (R1, R10): read the
/// current state of both sides.
fn filled(conflict: &Conflict, sides: &[SideIo]) -> Conflict {
    let mut conflict = conflict.clone();
    if conflict.a.is_none() && conflict.b.is_none() && conflict.duplicates.is_none() {
        conflict.a = signature_of(&sides[0], &conflict.rel).ok().flatten();
        conflict.b = signature_of(&sides[1], &conflict.rel).ok().flatten();
    }
    conflict
}

fn actions_for(
    group: CompareGroup,
    kind: ItemKind,
    conflicts: &[&Conflict],
    can_merge: bool,
) -> Vec<ItemAction> {
    let both = !conflicts.is_empty() && conflicts.iter().all(|c| c.a.is_some() && c.b.is_some());
    match (group, kind) {
        (CompareGroup::Conflict, ItemKind::Key) => {
            vec![ItemAction::KeepVersionA, ItemAction::KeepVersionB]
        }
        // R3: the node files of two folders of one name name two IDs.
        (CompareGroup::Conflict, ItemKind::Folder)
            if both && conflicts.iter().any(|c| c.rel.ends_with("/dir.c9r")) =>
        {
            vec![ItemAction::MergeFolders, ItemAction::KeepFoldersBoth]
        }
        (CompareGroup::Conflict, ItemKind::File) if both => {
            let mut actions = vec![ItemAction::KeepA, ItemAction::KeepB];
            // Keep-both and the merge work on one file with a conflict.
            if conflicts.len() == 1 {
                actions.push(ItemAction::KeepBoth);
                if can_merge {
                    actions.push(ItemAction::Merge);
                }
            }
            actions
        }
        (CompareGroup::Conflict, _) => vec![ItemAction::KeepA, ItemAction::KeepB],
        (
            CompareGroup::ChangedA
            | CompareGroup::ChangedB
            | CompareGroup::NewA
            | CompareGroup::NewB,
            _,
        ) => vec![ItemAction::Transfer],
        (CompareGroup::DeletedA | CompareGroup::DeletedB | CompareGroup::Deferred, _) => {
            vec![ItemAction::ConfirmDelete, ItemAction::Restore]
        }
        (CompareGroup::Structure, _) => Vec::new(),
    }
}

/// The entries of the preview (groups other than `structure`).
pub(super) fn file_items(
    job: &SyncJob,
    preview: &VaultPreview,
    sides: &[SideIo],
    maps: &[DirMap],
    session: &VaultSession,
) -> Vec<Held> {
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
    let mut add =
        |rel: &str, group: CompareGroup, action: Option<Action>, conflict: Option<Conflict>| {
            let located = locate(rel, maps);
            let entry = acc.entry(located.key.clone()).or_insert_with(|| Acc {
                located,
                files: Vec::new(),
            });
            entry.files.push(FileEntry {
                rel: rel.to_string(),
                group,
                action,
                conflict,
            });
        };
    for action in &preview.preview.actions {
        let group = group_of(action, &preview.deferred, sides);
        add(action_rel(action), group, Some(action.clone()), None);
    }
    for conflict in &preview.preview.conflicts {
        let conflict = filled(conflict, sides);
        add(
            &conflict.rel.clone(),
            CompareGroup::Conflict,
            None,
            Some(conflict),
        );
    }
    let combo = session.config().combo;
    let mut helds = Vec::with_capacity(acc.len());
    for (_, mut entry) in acc {
        entry.files.sort_by(|left, right| left.rel.cmp(&right.rel));
        let main_files: Vec<&FileEntry> = entry
            .files
            .iter()
            .filter(|file| !is_technical(&file.rel))
            .collect();
        let deciding: Vec<&FileEntry> = if main_files.is_empty() {
            entry.files.iter().collect()
        } else {
            main_files
        };
        let group = deciding
            .iter()
            .map(|file| file.group)
            .max_by_key(|group| group.strength())
            .unwrap_or(CompareGroup::ChangedA);
        let main = deciding
            .first()
            .map_or_else(String::new, |file| file.rel.clone());
        let located = &entry.located;
        let conflicts: Vec<&Conflict> = entry
            .files
            .iter()
            .filter_map(|file| file.conflict.as_ref())
            .collect();
        let can_merge = group == CompareGroup::Conflict
            && located.kind == ItemKind::File
            && conflicts.len() == 1
            && mergeable(sides, session, conflicts[0]);
        let finding = (group == CompareGroup::Conflict && located.kind == ItemKind::Key)
            .then(|| "KeyFileDiffers".to_string());
        let kind_key = located.kind.key();
        let identity = if located.path.is_empty() {
            main.clone()
        } else {
            located.path.clone()
        };
        let item = CompareItem {
            id: item_id(&job.id, kind_key, &identity),
            group,
            kind: located.kind,
            path: located.path.clone(),
            a: side_state(&sides[0], &main, located.kind, combo),
            b: side_state(&sides[1], &main, located.kind, combo),
            actions: actions_for(group, located.kind, &conflicts, can_merge),
            finding,
            note: located.note.clone(),
        };
        let rels = entry.files.iter().map(|file| file.rel.clone()).collect();
        let actions = entry
            .files
            .iter()
            .filter_map(|file| file.action.clone())
            .collect();
        let conflicts = entry
            .files
            .iter()
            .filter_map(|file| file.conflict.clone())
            .collect();
        helds.push(Held {
            item,
            payload: Payload::Files {
                actions,
                conflicts,
                rels,
                main,
            },
            done: false,
        });
    }
    helds
}
