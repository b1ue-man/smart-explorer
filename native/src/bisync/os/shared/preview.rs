use crate::vfs::Backend;
use std::io;
use std::sync::atomic::AtomicBool;

use super::guards::DeleteCounts;
use super::incremental::SyncEndpoints;
use super::keys::Spellings;
use super::omissions::SyncOmissions;
use super::run_types::{RunBlock, RunSettings, StateKey};
use super::snapshot::WalkFilter;
use super::snapshot_pair::read_pair;
use super::types::{Action, Baseline, BisyncOptions, BisyncStats, Conflict};

/// Read-only comparison with the same planner and omitted-subtree protection
/// as a run. Literal I/O spellings and options stay attached to its actions.
#[derive(Default)]
pub struct Preview {
    pub actions: Vec<Action>,
    pub conflicts: Vec<Conflict>,
    pub a_files: usize,
    pub b_files: usize,
    pub error: Option<String>,
    pub omissions: SyncOmissions,
    pub duplicate_removals: u64,
    pub blocked: Option<RunBlock>,
    pub state: Option<StateKey>,
    pub planned: Baseline,
    pub spellings: Spellings,
    pub dirs: Vec<super::DirAction>,
    pub options: Option<BisyncOptions>,
}

pub fn preview(
    a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str,
    opts: BisyncOptions, cancel: &AtomicBool, filter: &WalkFilter,
) -> Preview {
    preview_with(a, root_a, b, root_b, opts, cancel, filter, RunSettings::default())
}

/// Compare using the recorded owner and guard settings of the requesting job.
#[allow(clippy::too_many_arguments)]
pub fn preview_with(
    a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str,
    opts: BisyncOptions, cancel: &AtomicBool, filter: &WalkFilter, settings: RunSettings,
) -> Preview {
    let endpoints = SyncEndpoints::new(a, root_a, b, root_b);
    let result = (|| -> io::Result<Preview> {
        crate::vfs::validate_sync_roots(a, root_a, b, root_b)?;
        let id = super::pair_lock_id(a, root_a, b, root_b);
        let _lock = super::PairLock::acquire_wait(&id, Default::default(), cancel)?;
        let replicas = super::replica::identify(endpoints, &settings, false)?;
        let key = replicas.key;
        if let Some(blocked) = replicas.blocked {
            return Ok(Preview { state: Some(key), blocked: Some(blocked), ..Preview::default() });
        }
        let keys = super::orchestration_plan::keys(endpoints);
        let (_, records, dirs) = super::checkpoint_journal::Journal::load(&key, keys)?;
        let path = super::baseline_file(&key)?;
        let base = if !path.try_exists()? && !path.with_extension("journal").try_exists()?
            && replicas.history.is_none() && !key.is_legacy()
        {
            super::load_baseline(&super::baseline_path(&key.pair_id))?
        } else { records.baseline };
        let mut snapshot = read_pair(endpoints, opts, cancel, filter, &base)
            .map_err(|(path, error)| io::Error::other(format!("{path}: {error}")))?;
        let empty = (snapshot.a.is_empty(), snapshot.b.is_empty());
        let mut planning_base = base.clone();
        for conflict in &snapshot.conflicts {
            planning_base.remove(&conflict.rel);
            snapshot.a.omissions.record_kind(&conflict.rel, super::OmissionKind::NameImpossibleOnTarget, false);
        }
        let ctx = super::orchestration_plan::context(endpoints, opts, dirs.as_ref());
        let mut plan = super::orchestration_plan::prepare(endpoints, &mut snapshot.a,
            &mut snapshot.b, &planning_base, &ctx, cancel);
        plan.conflicts.extend(snapshot.conflicts);
        let mut deletes = DeleteCounts::of(&plan.actions);
        let mut duplicate_removals = 0u64;
        for repair in &snapshot.repairs {
            if let Some(group) = &repair.duplicates {
                let a = group.a.len().saturating_sub(1) as u64;
                let b = group.b.len().saturating_sub(1) as u64;
                deletes.add(super::PairSide::A, a);
                deletes.add(super::PairSide::B, b);
                plan.files_a = plan.files_a.saturating_add(a);
                plan.files_b = plan.files_b.saturating_add(b);
                duplicate_removals = duplicate_removals.saturating_add(a).saturating_add(b);
            }
        }
        // Mirror orphan duplicates take part in the same preflight count as
        // the real run. No mutation occurs while building the preview.
        let target = match (opts.delete, opts.direction) {
            (super::DeletePolicy::Mirror, super::Direction::AtoB) if !opts.move_files => Some((b, root_b, super::PairSide::B, &snapshot.a.tree)),
            (super::DeletePolicy::Mirror, super::Direction::BtoA) if !opts.move_files => Some((a, root_a, super::PairSide::A, &snapshot.b.tree)),
            _ => None,
        };
        if let Some((backend, root, side, source)) = target {
            let source_keys: std::collections::BTreeSet<_> = source.keys().map(|rel| keys.key(rel).into_owned()).collect();
            let repair_keys: std::collections::BTreeSet<_> = snapshot.repairs.iter()
                .map(|repair| keys.key(&repair.rel).into_owned()).collect();
            let candidates = backend.plan_dedupe_recursive(root, &|rel| {
                source_keys.contains(keys.key(rel).as_ref()) || plan.omissions.protects(rel)
            })?;
            let candidates: Vec<_> = candidates.iter().filter(|entry| {
                let rel = super::paths::rel_of(&entry.path, root);
                !plan.omissions.protects(&rel) && !repair_keys.contains(keys.key(&rel).as_ref())
            }).collect();
            let count = candidates.len() as u64;
            let orphans: std::collections::BTreeSet<_> = candidates.iter()
                .map(|entry| keys.key(&super::paths::rel_of(&entry.path, root)).into_owned())
                .filter(|key| !source_keys.contains(key)).collect();
            let extra = count.saturating_sub(orphans.len() as u64);
            deletes.add(side, extra);
            match side {
                super::PairSide::A => plan.files_a = plan.files_a.saturating_add(extra),
                super::PairSide::B => plan.files_b = plan.files_b.saturating_add(extra),
            }
            duplicate_removals = duplicate_removals.saturating_add(count);
        }
        let mut blocked = super::orchestration_plan::blocked(&plan, &snapshot.a, &snapshot.b,
            &base, &opts, &settings, deletes);
        if let Some(history) = replicas.history {
            for (side, empty, previous) in [(super::PairSide::A, empty.0, history.entries_a),
                (super::PairSide::B, empty.1, history.entries_b)]
            {
                if empty && previous > 0 && blocked.is_none() {
                    blocked = Some(RunBlock::SideEmpty { side, previous });
                }
            }
        }
        let planned = super::orchestration_plan::planned_signatures(&plan, &snapshot.a, &snapshot.b);
        Ok(Preview { actions: plan.actions, conflicts: plan.conflicts,
            a_files: plan.files_a as usize, b_files: plan.files_b as usize, error: None,
            omissions: plan.omissions, duplicate_removals, blocked, state: Some(key), planned,
            spellings: plan.spellings, dirs: plan.dirs, options: Some(opts) })
    })();
    result.unwrap_or_else(|error| Preview { error: Some(error.to_string()), ..Preview::default() })
}

/// Apply exactly one displayed action, against its captured state, under the
/// pair lock. Completed work is journaled before this function returns.
#[allow(clippy::too_many_arguments)]
pub fn apply_preview_action(
    a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str,
    preview: &Preview, action: &Action, opts: BisyncOptions, cancel: &AtomicBool,
) -> io::Result<BisyncStats> {
    if let Some(error) = &preview.error { return Err(io::Error::other(error.clone())); }
    if let Some(blocked) = &preview.blocked { return Err(io::Error::new(io::ErrorKind::PermissionDenied, blocked.message())); }
    if !preview.actions.contains(action) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Aktion gehört nicht zur Vorschau"));
    }
    let key = preview.state.as_ref().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Vorschau hat keinen Zustandsbezug"))?;
    let expected = preview.planned.get(super::core::action_rel(action)).copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Vorschau enthält keine Signaturen dieser Aktion"))?;
    let endpoints = SyncEndpoints::new(a, root_a, b, root_b);
    let lock = super::PairLock::acquire(&key.lock_id)?;
    super::single_recorded::validate_state(endpoints, key)?;
    super::single_recorded::apply_one(endpoints, &lock, key, action, expected, &preview.spellings,
        preview.options.unwrap_or(opts), cancel).map(|(stats, _)| stats)
}
