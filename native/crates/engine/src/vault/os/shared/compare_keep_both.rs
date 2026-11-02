//! „Beide behalten“ in a vault (vault plan C6.4, G26, R10, U26-6): the
//! engine's keep-both would invent a cipher name that decrypts to nothing,
//! so the vault comparison does it step by step, under the pair lock:
//!
//! 1. the conflict name `<Name> (Konflikt <JJJJ-MM-TT HHMM> <Bezeichnung>).<Ext>`
//!    in the folder of the original, encrypted with the folder's directory ID
//!    (`.c9s` when it exceeds the threshold);
//! 2. the cipher bytes of the losing version, unchanged, published under that
//!    name on the losing side only (stage and no-replace promotion; the file
//!    header carries the content key, the name enters only as the associated
//!    data of the name itself, so the copy decrypts: assumption A-U26-1);
//! 3. the winner (the newer version, side A on a tie) takes the original name
//!    through the engine (`resolve_recorded`, the loser goes to the versions);
//! 4. a new preview and the engine apply the new name to the other side and
//!    record the baseline, so the next run finds nothing left to do.
use std::io;
use std::sync::atomic::AtomicBool;

use crate::bisync::{resolve_recorded, Action};
use crate::cryptomator::{
    encrypt_name, needs_shortening, shortened_name, CONTENTS_FILE, INFLATED_NAME_FILE,
};
use crate::syncjobs::SyncJob;
use crate::vault::core::conflict_names::conflict_name;
use crate::vault::core::path_map::{child, storage_path};
use crate::vault::core::sync_rules::{action_rel, content_folder, phase_of};
use crate::vault::core::VaultSession;

use super::compare::invalid;
use super::compare_actions::{files_of, job_of, lock_preview, pair, pair_locks, ActionEnv};
use super::compare_cache::{stale, CompareState, SideIo};
use super::compare_files::{copy_new, put_new, remove_tree};
use super::sync_job::{apply_vault_preview_action, preview_vault_job_in};

/// A name in a content folder as the storage keeps it.
pub(super) struct NewEntry {
    /// The full encrypted name (`….c9r`).
    pub(super) encrypted: String,
    /// The stored name: the encrypted name, or its `….c9s` digest.
    pub(super) stored: String,
    pub(super) shortened: bool,
}

impl NewEntry {
    /// The entry in the content folder `content`, relative to the vault root.
    pub(super) fn node(&self, content: &str) -> String {
        child(content, &self.stored)
    }
}

/// The stored form of the cleartext name `name` in the folder with the
/// directory ID `parent_id`.
pub(super) fn new_entry(
    session: &VaultSession,
    parent_id: &str,
    name: &str,
) -> io::Result<NewEntry> {
    let encrypted = session.with_key(|key| encrypt_name(key, name, parent_id))?;
    let shortened = needs_shortening(&encrypted, session.config().shortening_threshold);
    let stored = if shortened {
        shortened_name(&encrypted)
    } else {
        encrypted.clone()
    };
    Ok(NewEntry {
        encrypted,
        stored,
        shortened,
    })
}

/// Whether `stored` exists in `content` on any side (an unreadable answer
/// counts as taken: nothing is replaced on a guess).
pub(super) fn entry_taken(sides: &[SideIo], content: &str, stored: &str) -> bool {
    sides.iter().any(|side| {
        side.backend
            .try_exists(&storage_path(&side.root, &child(content, stored)))
            .unwrap_or(true)
    })
}

/// Publishes the cipher bytes at `source` (relative to the vault root) as
/// the new entry on `side`; a half-made `.c9s` folder is removed again.
pub(super) fn publish_copy(
    side: &SideIo,
    content: &str,
    entry: &NewEntry,
    source: &str,
) -> io::Result<()> {
    let inner = side.backend.as_ref();
    let source = storage_path(&side.root, source);
    let node = storage_path(&side.root, &entry.node(content));
    if !entry.shortened {
        return copy_new(inner, &source, &node);
    }
    inner.create_dir_new(&node)?;
    let made = put_new(
        inner,
        &child(&node, INFLATED_NAME_FILE),
        entry.encrypted.as_bytes(),
    )
    .and_then(|()| copy_new(inner, &source, &child(&node, CONTENTS_FILE)));
    if made.is_err() {
        let _ = remove_tree(inner, &node);
    }
    made
}

/// The engine applies the actions of the new entry of a fresh preview.
fn apply_new_entry(
    env: &ActionEnv<'_>,
    state: &CompareState,
    job: &SyncJob,
    prefix: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (a, b) = pair(state)?;
    let fresh = preview_vault_job_in(env.store, env.resolve, job, cancel);
    if let Some(reason) = fresh
        .unavailable
        .clone()
        .or_else(|| fresh.preview.error.clone())
    {
        return Err(io::Error::other(reason));
    }
    let below = format!("{prefix}/");
    let mut todo: Vec<Action> = fresh
        .preview
        .actions
        .iter()
        .filter(|action| {
            let rel = action_rel(action);
            rel == prefix || rel.starts_with(&below)
        })
        .cloned()
        .collect();
    todo.sort_by_key(|action| (phase_of(action), action_rel(action).to_string()));
    for (position, action) in todo.iter().enumerate() {
        apply_vault_preview_action(
            a.backend.as_ref(),
            &a.root,
            b.backend.as_ref(),
            &b.root,
            &fresh,
            &todo[position + 1..],
            action,
            job,
            cancel,
        )?;
    }
    Ok(())
}

/// „Beide behalten“ for the conflict of the entry `index`.
pub(super) fn keep_both(
    env: &ActionEnv<'_>,
    state: &CompareState,
    index: usize,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (a, b) = pair(state)?;
    let job = job_of(state)?;
    let (_, conflicts, _) = files_of(state, index)?;
    let [conflict] = conflicts.as_slice() else {
        return Err(invalid("Beide behalten gilt für eine Datei im Konflikt"));
    };
    let (Some(sig_a), Some(sig_b)) = (conflict.a, conflict.b) else {
        return Err(invalid("Beide behalten braucht beide Fassungen"));
    };
    // The newer version keeps the name; side A on a tie.
    let keep_a = sig_a.mtime_ms >= sig_b.mtime_ms;
    let loser = if keep_a { b } else { a };
    let path = state
        .items()
        .get(index)
        .map(|held| held.item.path.clone())
        .ok_or_else(stale)?;
    let (_, name) = path
        .rsplit_once('/')
        .ok_or_else(|| invalid("Der Eintrag hat keinen Pfad"))?;
    let content = content_folder(&conflict.rel)
        .ok_or_else(|| invalid("Die Datei liegt nicht im Datenordner des Tresors"))?
        .to_string();
    let parent_id = state
        .maps
        .iter()
        .find_map(|map| map.dirs.get(&content))
        .map(|dir| dir.id.clone())
        .ok_or_else(|| invalid("Der Ordner des Eintrags ist unbekannt"))?;
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M").to_string();
    let taken = |candidate: &str| match new_entry(&state.session, &parent_id, candidate) {
        Ok(entry) => entry_taken(&state.sides, &content, &entry.stored),
        Err(_) => true,
    };
    let new_name = conflict_name(name, &stamp, &loser.label, &taken);
    let entry = new_entry(&state.session, &parent_id, &new_name)?;
    {
        let _locks = pair_locks(env, state)?;
        publish_copy(loser, &content, &entry, &conflict.rel)?;
    }
    {
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
            conflict,
            keep_a,
            None,
            key,
            cancel,
            |_phase| (),
        )?;
    }
    apply_new_entry(env, state, job, &entry.node(&content), cancel)
}
