//! The rules of vault sync jobs (vault plan C5.4 R1–R11) on the encrypted
//! storage of two vault sites. The engine is the general bisync engine
//! (C5.3); these rules reach it through `bisync::plan_rules::enter` on the
//! thread that runs, previews or applies, and every other job kind plans
//! and applies exactly as before.
//!
//! - Rule stage (`adjust`, after planning, for full scans and previews):
//!   R1 the key file and the configuration are never deleted, the deletion
//!   becomes a conflict; R2 a content folder deleted on one side and changed
//!   on the other is left alone, and the node deletions of the deleting side
//!   wait; R9 stage names of se never travel; R10 keep-both actions become
//!   conflicts (they cannot arise under the fixed strict mode, the stage
//!   protects the cipher names anyway); a copy still being seeded only
//!   receives (C5.9); R12 no node folder is created empty, and one whose
//!   files a run deleted is removed (`sync_rules_dirs.rs`).
//! - Apply phases (R4): phase 1 creates and changes every entry that is no
//!   node file and deletes node files; phase 2 creates and changes node
//!   files and deletes entries below `d/`. Without the masterkey a content
//!   folder cannot be mapped to its node, so after an incomplete phase-1
//!   action every node creation waits, and deletions below `d/` wait in the
//!   same content folder or after any unfinished node deletion.
//!
//! R3 and R5 (both sides new or changed with different bytes) are conflicts
//! of the fixed strict mode; R6 are the job's deletion stops; R7 is checked
//! before planning (`os/shared/sync_job.rs`); R8 and R11 need no rule.
//! Pure decisions on relative cipher paths: no I/O and no key, so a vault
//! job runs while the vault is locked.
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::bisync::plan_rules::{Action, PairPlan, PlanRules};
use crate::bisync::{Conflict, DirAction, PairSide};
use crate::cryptomator::{
    is_metadata_name, CIPHER_SUFFIX, DATA_DIR, DIR_FILE, DIR_ID_BACKUP_FILE, MASTERKEY_FILE,
    SHORTENED_SUFFIX, SYMLINK_FILE, VAULT_CONFIG_FILE,
};

/// Reason of an action R2 leaves for the comparison mode.
pub(crate) const DEFERRED_FOLDER_CONFLICT: &str =
    "Tresor: Ordner auf einer Seite gelöscht, auf der anderen geändert – im Vergleichsmodus entscheiden";
/// Reason of a node deletion R2 holds back.
pub(crate) const DEFERRED_NODE_DELETION: &str =
    "Tresor: Knotenlöschung wartet, bis der gelöschte Ordner im Vergleichsmodus geklärt ist";

/// What a relative path of the encrypted storage is (format 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CipherEntry<'a> {
    /// `vault.cryptomator`, `masterkey.cryptomator` or a `.bkup` of them in
    /// the vault root (R1).
    Metadata,
    /// The node file (`dir.c9r`, `symlink.c9r`) of a folder or a link in
    /// the content folder `content` (`d/XX/YYYY…`).
    Node { content: &'a str },
    /// Every other entry below `d/`: files, `dirid.c9r`, `contents.c9r`,
    /// `name.c9s`, in the content folder `content`.
    Content { content: &'a str },
    /// Anything outside `d/` that is no metadata.
    Other,
}

/// Classifies `rel` (slash-separated, relative to the vault root).
pub(crate) fn classify(rel: &str) -> CipherEntry<'_> {
    let rel = rel.trim_start_matches('/');
    if !rel.contains('/') && is_metadata_name(rel) {
        return CipherEntry::Metadata;
    }
    let Some(rest) = rel
        .strip_prefix(DATA_DIR)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return CipherEntry::Other;
    };
    let parts: Vec<&str> = rest.split('/').collect();
    // `d/XX/YYYY…` is the content folder (`parts[0]`, `parts[1]`); a
    // shorter path stays whole.
    let content = match parts.as_slice() {
        [two, rest_of_hash, ..] => &rel[..DATA_DIR.len() + 1 + two.len() + 1 + rest_of_hash.len()],
        _ => rel,
    };
    if parts.len() == 4 {
        let (holder, name) = (parts[2], parts[3]);
        let node_holder = holder.ends_with(CIPHER_SUFFIX) || holder.ends_with(SHORTENED_SUFFIX);
        if node_holder && (name == DIR_FILE || name == SYMLINK_FILE) {
            return CipherEntry::Node { content };
        }
    }
    CipherEntry::Content { content }
}

/// The content folder of `rel`, for entries below `d/`.
pub(crate) fn content_folder(rel: &str) -> Option<&str> {
    match classify(rel) {
        CipherEntry::Node { content } | CipherEntry::Content { content } => Some(content),
        CipherEntry::Metadata | CipherEntry::Other => None,
    }
}

/// Whether `rel` is the `dirid.c9r` of its content folder.
pub(crate) fn is_dir_id_backup(rel: &str) -> bool {
    content_folder(rel).is_some_and(|content| {
        rel.trim_start_matches('/')
            .strip_prefix(content)
            .is_some_and(|rest| rest == format!("/{DIR_ID_BACKUP_FILE}"))
    })
}

fn is_node(rel: &str) -> bool {
    matches!(classify(rel), CipherEntry::Node { .. })
}

/// The relative path of an action.
pub(crate) fn action_rel(action: &Action) -> &str {
    match action {
        Action::CopyAtoB(rel)
        | Action::CopyBtoA(rel)
        | Action::FinalizeMoveAtoB(rel)
        | Action::FinalizeMoveBtoA(rel)
        | Action::DeleteA(rel)
        | Action::DeleteB(rel)
        | Action::KeepBothAtoB(rel)
        | Action::KeepBothBtoA(rel) => rel,
    }
}

/// The side a deletion removes from; `None` for every other action.
fn deleted_on(action: &Action) -> Option<PairSide> {
    match action {
        Action::DeleteA(_) => Some(PairSide::A),
        Action::DeleteB(_) => Some(PairSide::B),
        _ => None,
    }
}

/// The side a copy writes to; `None` for every other action.
fn copied_to(action: &Action) -> Option<PairSide> {
    match action {
        Action::CopyAtoB(_) | Action::FinalizeMoveAtoB(_) | Action::KeepBothAtoB(_) => {
            Some(PairSide::B)
        }
        Action::CopyBtoA(_) | Action::FinalizeMoveBtoA(_) | Action::KeepBothBtoA(_) => {
            Some(PairSide::A)
        }
        _ => None,
    }
}

/// The apply phase of an action (R4): 1 or 2.
pub(crate) fn phase_of(action: &Action) -> u8 {
    let rel = action_rel(action);
    match (deleted_on(action), classify(rel)) {
        (Some(_), CipherEntry::Node { .. }) => 1,
        (Some(_), CipherEntry::Content { .. }) => 2,
        (Some(_), _) => 1,
        (None, CipherEntry::Node { .. }) => 2,
        (None, _) => 1,
    }
}

/// Whether `rel` is a component path containing a stage name of se (R9).
fn has_stage_name(rel: &str) -> bool {
    rel.split('/').any(crate::vfs::is_staging_name)
}

/// The key file or the configuration of the vault (never deleted, R1).
fn is_key_metadata(rel: &str) -> bool {
    let rel = rel.trim_start_matches('/');
    rel == MASTERKEY_FILE || rel == VAULT_CONFIG_FILE
}

type ActionKey = (u8, String);

fn key_of(action: &Action) -> ActionKey {
    let kind = match action {
        Action::CopyAtoB(_) => 0,
        Action::CopyBtoA(_) => 1,
        Action::FinalizeMoveAtoB(_) => 2,
        Action::FinalizeMoveBtoA(_) => 3,
        Action::DeleteA(_) => 4,
        Action::DeleteB(_) => 5,
        Action::KeepBothAtoB(_) => 6,
        Action::KeepBothBtoA(_) => 7,
    };
    (kind, action_rel(action).to_string())
}

/// The rules of one vault job run.
#[derive(Debug, Default)]
pub(crate) struct VaultRules {
    /// The side that is a copy still being seeded (C5.9): it only receives.
    seeding: Option<PairSide>,
    /// Actions R2 leaves out of this run, with their reason.
    deferred: Mutex<Vec<(ActionKey, &'static str)>>,
    /// How many plans the rule stage saw (a full scan or a preview).
    adjusted: AtomicUsize,
}

impl VaultRules {
    pub(crate) fn new(seeding: Option<PairSide>) -> Self {
        Self {
            seeding,
            ..Self::default()
        }
    }

    /// How many plans the rule stage adjusted: every run of a vault job
    /// plans with a full scan (R4a), the incremental mirror never does.
    pub(crate) fn adjusted(&self) -> usize {
        self.adjusted.load(Ordering::Acquire)
    }

    fn deferred(&self) -> std::sync::MutexGuard<'_, Vec<(ActionKey, &'static str)>> {
        self.deferred.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The reason R2 gave `action`, if it holds it back.
    pub(crate) fn rule_reason(&self, action: &Action) -> Option<&'static str> {
        let key = key_of(action);
        self.deferred()
            .iter()
            .find(|(known, _)| *known == key)
            .map(|(_, reason)| *reason)
    }
}

/// A conflict without the signatures of the planner (the rule stage sees
/// no trees); consumers read the current state again (C6).
fn conflict(rel: &str) -> Conflict {
    Conflict {
        rel: rel.to_string(),
        a: None,
        b: None,
        duplicates: None,
    }
}

/// Seeding (C5.9): the copy `seeding` only receives. A deletion on the other
/// side is a file the copy has not received yet (or lost): it is copied
/// again. A conflict where only the copy lacks the file is a copy as well;
/// real divergences stay conflicts.
fn seed_only(plan: &mut PairPlan, seeding: PairSide) {
    let source = seeding.other();
    for action in &mut plan.actions {
        if deleted_on(action) == Some(source) {
            let rel = action_rel(action).to_string();
            *action = match source {
                PairSide::A => Action::CopyAtoB(rel),
                PairSide::B => Action::CopyBtoA(rel),
            };
        }
    }
    let mut kept = Vec::with_capacity(plan.conflicts.len());
    for item in std::mem::take(&mut plan.conflicts) {
        let (present, missing) = match seeding {
            PairSide::B => (item.a, item.b),
            PairSide::A => (item.b, item.a),
        };
        if present.is_some() && missing.is_none() && item.duplicates.is_none() {
            plan.actions.push(match source {
                PairSide::A => Action::CopyAtoB(item.rel.clone()),
                PairSide::B => Action::CopyBtoA(item.rel.clone()),
            });
        } else {
            kept.push(item);
        }
    }
    plan.conflicts = kept;
    plan.dirs
        .retain(|dir| !matches!(dir, DirAction::Remove { side, .. } if *side == source));
}

/// R2: content folders whose `dirid.c9r` one side deleted while the other
/// changed something in them, with the side that deleted.
fn folder_conflicts(plan: &PairPlan) -> Vec<(String, PairSide)> {
    let mut found = Vec::new();
    for action in &plan.actions {
        let Some(removed_on) = deleted_on(action) else {
            continue;
        };
        let rel = action_rel(action);
        if !is_dir_id_backup(rel) {
            continue;
        }
        let Some(content) = content_folder(rel) else {
            continue;
        };
        // The side that still has the folder changed something in it: a
        // copy from it to the deleting side, or a conflict.
        let deleting = removed_on.other();
        let changed = plan.actions.iter().any(|other| {
            copied_to(other) == Some(deleting) && content_folder(action_rel(other)) == Some(content)
        }) || plan
            .conflicts
            .iter()
            .any(|item| content_folder(&item.rel) == Some(content));
        if changed && !found.iter().any(|(known, _)| known == content) {
            found.push((content.to_string(), deleting));
        }
    }
    found
}

impl PlanRules for VaultRules {
    fn adjust(&self, plan: &mut PairPlan) {
        self.adjusted.fetch_add(1, Ordering::AcqRel);
        // R9: stage names never travel (the walk leaves them out already).
        plan.actions
            .retain(|action| !has_stage_name(action_rel(action)));
        plan.conflicts.retain(|item| !has_stage_name(&item.rel));
        // R10 and R1: keep-both actions and deletions of the key file or the
        // configuration become conflicts.
        let mut actions = Vec::with_capacity(plan.actions.len());
        for action in std::mem::take(&mut plan.actions) {
            let rel = action_rel(&action);
            let keep_both = matches!(action, Action::KeepBothAtoB(_) | Action::KeepBothBtoA(_));
            if keep_both || (deleted_on(&action).is_some() && is_key_metadata(rel)) {
                if !plan.conflicts.iter().any(|item| item.rel == rel) {
                    plan.conflicts.push(conflict(rel));
                }
            } else {
                actions.push(action);
            }
        }
        plan.actions = actions;
        if let Some(seeding) = self.seeding {
            seed_only(plan, seeding);
        }
        // R2: leave the folder alone, hold the node deletions of the side
        // that deleted it; no empty folder is created inside it.
        let folders = folder_conflicts(plan);
        let mut deferred = Vec::new();
        for action in &plan.actions {
            let rel = action_rel(action);
            let in_folder = content_folder(rel)
                .is_some_and(|content| folders.iter().any(|(known, _)| known == content));
            let node_deletion = is_node(rel)
                && deleted_on(action).is_some_and(|side| {
                    folders.iter().any(|(_, deleting)| side == deleting.other())
                });
            if in_folder {
                deferred.push((key_of(action), DEFERRED_FOLDER_CONFLICT));
            } else if node_deletion {
                deferred.push((key_of(action), DEFERRED_NODE_DELETION));
            }
        }
        plan.dirs.retain(|dir| {
            !matches!(dir, DirAction::Create { .. })
                || !folders.iter().any(|(content, _)| {
                    dir.rel() == content || dir.rel().starts_with(&format!("{content}/"))
                })
        });
        // R12: node folders are never created empty, and emptied ones go.
        let deletions: Vec<(PairSide, String)> = plan
            .actions
            .iter()
            .filter(|action| !deferred.iter().any(|(key, _)| *key == key_of(action)))
            .filter_map(|action| Some((deleted_on(action)?, action_rel(action).to_string())))
            .collect();
        let writes: Vec<(PairSide, String)> = plan
            .actions
            .iter()
            .filter_map(|action| Some((copied_to(action)?, action_rel(action).to_string())))
            .collect();
        super::sync_rules_dirs::node_folders(&mut plan.dirs, &deletions, &writes);
        *self.deferred() = deferred;
    }

    fn phases(&self, actions: &[Action]) -> Vec<Vec<usize>> {
        let deferred: BTreeSet<ActionKey> =
            self.deferred().iter().map(|(key, _)| key.clone()).collect();
        let (mut first, mut second) = (Vec::new(), Vec::new());
        for (index, action) in actions.iter().enumerate() {
            if deferred.contains(&key_of(action)) {
                continue;
            }
            if phase_of(action) == 1 {
                first.push(index);
            } else {
                second.push(index);
            }
        }
        vec![first, second]
    }

    fn held_back(&self, incomplete: &[&Action], coming: &[&Action]) -> Vec<usize> {
        let deferred: BTreeSet<ActionKey> =
            self.deferred().iter().map(|(key, _)| key.clone()).collect();
        // Failed or held actions (not those R2 leaves out on purpose).
        let failed = incomplete
            .iter()
            .any(|action| !deferred.contains(&key_of(action)));
        let node_deletion_open = incomplete
            .iter()
            .any(|action| deleted_on(action).is_some() && is_node(action_rel(action)));
        let open_folders: BTreeSet<&str> = incomplete
            .iter()
            .filter_map(|action| content_folder(action_rel(action)))
            .collect();
        coming
            .iter()
            .enumerate()
            .filter(|(_, action)| {
                let rel = action_rel(action);
                match (deleted_on(action), classify(rel)) {
                    (None, CipherEntry::Node { .. }) => failed,
                    (Some(_), CipherEntry::Content { content }) => {
                        node_deletion_open || open_folders.contains(content)
                    }
                    _ => false,
                }
            })
            .map(|(position, _)| position)
            .collect()
    }

    fn deferral_reason(&self, action: &Action) -> String {
        self.rule_reason(action)
            .unwrap_or(crate::bisync::plan_rules::DEFERRED_BY_RULE)
            .to_string()
    }
}

/// Whether one action of a vault preview may be applied on its own now
/// (C6.4 „Jetzt übertragen“ through `bisync::apply_preview_action`, which
/// has no phases): an action of phase 2 waits while an action of phase 1 it
/// depends on is still `pending` (the rules of `held_back`).
pub(crate) fn order_allows(pending: &[Action], action: &Action) -> Result<(), &'static str> {
    if phase_of(action) == 1 {
        return Ok(());
    }
    let earlier: Vec<&Action> = pending
        .iter()
        .filter(|other| phase_of(other) == 1 && *other != action)
        .collect();
    if VaultRules::default()
        .held_back(&earlier, &[action])
        .is_empty()
    {
        Ok(())
    } else {
        Err(crate::bisync::plan_rules::HELD_BACK)
    }
}

#[cfg(test)]
#[path = "sync_rules_tests.rs"]
mod tests;
