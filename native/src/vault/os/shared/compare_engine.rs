//! The actions on an entry that the engine of the sync does (vault plan
//! C6.4, U26-5): „Fassung A/B behalten“, „Jetzt übertragen“, „Löschung
//! übernehmen“ and „Wiederherstellen“. `resolve_recorded` and
//! `apply_preview_action` take the pair lock themselves and record the
//! result in the job's state, so a later run finds nothing left to do.
use std::io;
use std::sync::atomic::AtomicBool;

use crate::bisync::{apply_preview_action, resolve_recorded, Action, BisyncStats, Conflict};
use crate::syncjobs::SyncJob;
use crate::vault::core::sync_rules::{
    action_rel, content_folder, is_dir_id_backup, order_allows, phase_of,
};
use crate::vfs::Backend;

use super::compare::{invalid, signature_of};
use super::compare_actions::{files_of, job_of, lock_preview, pair};
use super::compare_cache::CompareState;
use super::sync_job::{apply_vault_preview_action, VaultPreview};

/// „Fassung A/B behalten“ for every conflict of the entry: the loser goes
/// to the versions of the engine, the result is recorded in the job's state.
pub(super) fn keep(
    state: &CompareState,
    index: usize,
    keep_a: bool,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (a, b) = pair(state)?;
    let (_, conflicts, _) = files_of(state, index)?;
    if conflicts.is_empty() {
        return Err(invalid("Dieser Eintrag hat keinen Konflikt"));
    }
    let preview = lock_preview(state)?;
    let key = preview
        .preview
        .state
        .as_ref()
        .ok_or_else(|| invalid("Die Vorschau hat keinen Zustandsbezug"))?;
    for conflict in &conflicts {
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
    Ok(())
}

/// The actions of the preview an action of `rels` waits for (U26-5): its
/// own open actions and those inside a content folder the entry owns (the
/// folder of its `dirid.c9r`). With the key at hand the other entries of the
/// comparison do not matter, unlike in a run without the key.
fn pending_for(state: &CompareState, preview: &VaultPreview, rels: &[String]) -> Vec<Action> {
    let owned: Vec<&str> = rels
        .iter()
        .filter(|rel| is_dir_id_backup(rel))
        .filter_map(|rel| content_folder(rel))
        .collect();
    let applied = state.applied();
    preview
        .preview
        .actions
        .iter()
        .enumerate()
        .filter(|(position, action)| {
            let rel = action_rel(action);
            !applied.get(*position).copied().unwrap_or(false)
                && (rels.iter().any(|own| own == rel)
                    || content_folder(rel).is_some_and(|folder| owned.contains(&folder)))
        })
        .map(|(_, action)| action.clone())
        .collect()
}

fn mark_applied(state: &CompareState, preview: &VaultPreview, action: &Action) {
    let mut applied = state.applied();
    if let Some(position) = preview
        .preview
        .actions
        .iter()
        .position(|planned| planned == action)
    {
        if let Some(flag) = applied.get_mut(position) {
            *flag = true;
        }
    }
}

fn in_order(mut actions: Vec<Action>) -> Vec<Action> {
    actions.sort_by_key(|action| (phase_of(action), action_rel(action).to_string()));
    actions
}

/// „Jetzt übertragen“: every action of the entry, phase 1 before phase 2
/// (R4); one that fails stops the rest, the error goes to the caller.
pub(super) fn transfer(state: &CompareState, index: usize, cancel: &AtomicBool) -> io::Result<()> {
    let (a, b) = pair(state)?;
    let job = job_of(state)?;
    let (actions, _, rels) = files_of(state, index)?;
    let preview = lock_preview(state)?;
    for action in in_order(actions) {
        let pending = pending_for(state, &preview, &rels);
        apply_vault_preview_action(
            a.backend.as_ref(),
            &a.root,
            b.backend.as_ref(),
            &b.root,
            &preview,
            &pending,
            &action,
            job,
            cancel,
        )?;
        mark_applied(state, &preview, &action);
    }
    Ok(())
}

/// One deletion the user confirmed: like `apply_vault_preview_action`, but
/// without the refusal of deferred actions; the order of R4 still holds.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_confirmed_deletion(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    preview: &VaultPreview,
    pending: &[Action],
    action: &Action,
    job: &SyncJob,
    cancel: &AtomicBool,
) -> io::Result<BisyncStats> {
    order_allows(pending, action)
        .map_err(|reason| io::Error::new(io::ErrorKind::WouldBlock, reason))?;
    let opts = job.checked_vault_opts(false).map_err(invalid)?;
    apply_preview_action(a, root_a, b, root_b, &preview.preview, action, opts, cancel)
}

/// „Löschung übernehmen“: the deletions of the entry, also those the rules
/// of the vault job hold back (R2).
pub(super) fn confirm_delete(
    state: &CompareState,
    index: usize,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (a, b) = pair(state)?;
    let job = job_of(state)?;
    let (actions, _, rels) = files_of(state, index)?;
    let deletions: Vec<Action> = actions
        .into_iter()
        .filter(|action| matches!(action, Action::DeleteA(_) | Action::DeleteB(_)))
        .collect();
    if deletions.is_empty() {
        return Err(invalid("Dieser Eintrag hat keine Löschung zu übernehmen"));
    }
    let preview = lock_preview(state)?;
    for action in in_order(deletions) {
        let pending = pending_for(state, &preview, &rels);
        apply_confirmed_deletion(
            a.backend.as_ref(),
            &a.root,
            b.backend.as_ref(),
            &b.root,
            &preview,
            &pending,
            &action,
            job,
            cancel,
        )?;
        mark_applied(state, &preview, &action);
    }
    Ok(())
}

/// „Wiederherstellen“: the version that exists is copied back to the side
/// that deleted (or did not receive) it, through the engine's resolution
/// with the current signatures of both sides.
pub(super) fn restore(state: &CompareState, index: usize, cancel: &AtomicBool) -> io::Result<()> {
    let (a, b) = pair(state)?;
    let (actions, conflicts, _) = files_of(state, index)?;
    let mut keeps: Vec<(String, bool)> = Vec::new();
    for action in &actions {
        let keep_a = matches!(
            action,
            Action::CopyAtoB(_)
                | Action::FinalizeMoveAtoB(_)
                | Action::KeepBothAtoB(_)
                | Action::DeleteA(_)
        );
        keeps.push((action_rel(action).to_string(), keep_a));
    }
    for conflict in &conflicts {
        if conflict.a.is_some() != conflict.b.is_some() {
            keeps.push((conflict.rel.clone(), conflict.a.is_some()));
        }
    }
    if keeps.is_empty() {
        return Err(invalid("Dieser Eintrag hat nichts wiederherzustellen"));
    }
    let preview = lock_preview(state)?;
    let key = preview
        .preview
        .state
        .as_ref()
        .ok_or_else(|| invalid("Die Vorschau hat keinen Zustandsbezug"))?;
    for (rel, keep_a) in keeps {
        let conflict = Conflict {
            a: signature_of(a, &rel)?,
            b: signature_of(b, &rel)?,
            rel,
            duplicates: None,
        };
        let keeper = if keep_a { conflict.a } else { conflict.b };
        if keeper.is_none() {
            // The version to restore is gone: nothing to bring back.
            continue;
        }
        resolve_recorded(
            a.backend.as_ref(),
            &a.root,
            b.backend.as_ref(),
            &b.root,
            &conflict,
            keep_a,
            None,
            key,
            cancel,
            |_phase| (),
        )?;
    }
    Ok(())
}
