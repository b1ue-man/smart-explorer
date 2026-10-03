//! Optional one-way mirror cache. The authoritative checkpoint always wins;
//! any untrusted cache, partial scan or ambiguous key selects the full planner.
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::Ordering;

use crate::vfs::Backend;

use super::checkpoint::ApplyScope;
use super::checkpoint_run::CheckpointSink;
use super::guards::{deletion_block, empty_side_block, unconfirmed, DeleteCounts};
use super::incremental_changes::{
    action_plan_for, apply_trees, target_touched_drifted_spelled,
};
use super::incremental_collect::{
    changes_from_backend, changes_from_source_walk_scoped, ChangeCollection,
};
use super::orchestration::{failure, Outcome, RunState};
use super::replica_state::index_id;
use super::state_metadata::{index_dirty_path, save_history, PairHistory};
use super::state_store::{ItemRecord, PairRecord, Side};
use super::types::{Baseline, BisyncOptions, DeletePolicy, Direction, PairSide};

#[path = "incremental_index_commit.rs"]
mod index_commit;
pub(super) use index_commit::{
    bootstrap_incremental_state, bootstrap_run, invalidate_incremental_state, retire_index,
};
use index_commit::{mode, open_store};

#[derive(Clone, Copy)]
pub(super) struct SyncEndpoints<'a> {
    pub(super) a: &'a dyn Backend,
    pub(super) root_a: &'a str,
    pub(super) b: &'a dyn Backend,
    pub(super) root_b: &'a str,
}

impl<'a> SyncEndpoints<'a> {
    pub(super) fn new(
        a: &'a dyn Backend,
        root_a: &'a str,
        b: &'a dyn Backend,
        root_b: &'a str,
    ) -> Self {
        Self {
            a,
            root_a,
            b,
            root_b,
        }
    }
}

pub(super) fn mirror_source<'a>(
    endpoints: SyncEndpoints<'a>,
    opts: BisyncOptions,
) -> Option<(&'a dyn Backend, &'a str, Side)> {
    let SyncEndpoints {
        a,
        root_a,
        b,
        root_b,
    } = endpoints;
    // A move has a second, source-deletion phase and cannot be represented by
    // the destination-only incremental mirror index. Use the full planner so a
    // committed copy with a failed delete becomes a verified FinalizeMove.
    if opts.delete != DeletePolicy::Mirror || opts.move_files {
        return None;
    }
    match opts.direction {
        Direction::AtoB => Some((a, root_a, Side::A)),
        Direction::BtoA => Some((b, root_b, Side::B)),
        Direction::Both => None,
    }
}

pub(super) fn try_incremental_run(state: &RunState<'_>) -> Option<Outcome> {
    match super::orchestration_plan::pending_paths(state.lock, state.key, state.endpoints) {
        Ok(pending) if pending.is_empty() => {}
        Ok(_) => return None,
        Err(error) => return Some(failure("Merge-Wiederanlauf", error)),
    }
    let endpoints = state.endpoints;
    let opts = state.opts;
    if opts.dry_run
        || state.history.is_none()
        || index_dirty_path(state.key).ok()?.try_exists().ok()?
    {
        return None;
    }
    if state.cancel.load(Ordering::Acquire) {
        return Some(Outcome::default());
    }
    let (source, source_root, source_side) = mirror_source(endpoints, opts)?;
    let (target, target_root) = if source_side == Side::A {
        (endpoints.b, endpoints.root_b)
    } else {
        (endpoints.a, endpoints.root_a)
    };
    let source_pair = if source_side == Side::A {
        PairSide::A
    } else {
        PairSide::B
    };
    let target_pair = source_pair.other();
    let keys = super::orchestration_plan::keys(endpoints);
    let pair = index_id(state.key).ok()?;
    let mut store = open_store(state.store_path).ok()?;
    let rec = store.load_pair(&pair).ok().flatten()?;
    if !record_matches(&rec, endpoints, source_side)
        || rec.mode != mode(state)
        || !root_id_matches(endpoints.a, endpoints.root_a, rec.root_a_id.as_deref())
        || !root_id_matches(endpoints.b, endpoints.root_b, rec.root_b_id.as_deref())
    {
        return None;
    }
    let (items_a, items_b) = store.load_pair_items(&pair).ok()?;
    if cache_by_key(&items_a, &items_b, keys)? != baseline_by_key(state.baseline, keys) {
        return None;
    }
    let (source_items, target_items) = if source_side == Side::A {
        (&items_a, &items_b)
    } else {
        (&items_b, &items_a)
    };
    let collection =
        if source.supports_changes() && state.settings.depth == super::ScanDepth::Incremental {
            changes_from_backend(
                &store,
                &rec,
                source,
                source_root,
                source_side,
                source_items,
                state.filter,
                state.cancel,
            )
        } else {
            changes_from_source_walk_scoped(
                source,
                source_root,
                target,
                opts,
                state.filter,
                source_items,
                state.cancel,
                state.dirs,
                keys,
            )
        };
    let (changes, cursor) = match collection {
        ChangeCollection::Ready {
            changes,
            new_cursor,
        } => (changes, new_cursor),
        ChangeCollection::Rebuild => return None,
        ChangeCollection::Canceled => {
            return Some(Outcome {
                baseline: state.baseline.clone(),
                ..Outcome::default()
            })
        }
    };
    // A filter transition must become a protected omission in the pair
    // planner. Advancing only a feed cursor would leave an old managed row
    // able to authorize a later deletion (e.g. a now-hidden Windows file).
    if changes.iter().any(|change| !change.managed) {
        return None;
    }
    // Case/NFC alias transitions and rename cycles that collapse onto the
    // same receiving object require the complete pair planner, never a later
    // deletion of a path a copy just published.
    let mut aliases = BTreeSet::new();
    for change in &changes {
        if !aliases.insert(keys.key(&change.rel).into_owned()) {
            return None;
        }
        if change
            .old_rel
            .as_deref()
            .is_some_and(|old| keys.key(old) == keys.key(&change.rel) && old != change.rel)
        {
            return None;
        }
    }
    let mut names = super::state_spellings::load(state.key, keys).ok()?;
    let planned = action_plan_for(source_side, &changes);
    let upsert_keys: BTreeSet<_> = planned
        .upserts
        .iter()
        .map(super::core::action_rel)
        .map(|rel| keys.key(rel).into_owned())
        .collect();
    if planned
        .deletes
        .iter()
        .map(super::core::action_rel)
        .any(|rel| upsert_keys.contains(keys.key(rel).as_ref()))
    {
        return None;
    }
    let actions: Vec<_> = planned
        .upserts
        .iter()
        .chain(&planned.deletes)
        .cloned()
        .collect();
    let spellings = names.for_actions(&actions, source_pair, keys);
    if target_touched_drifted_spelled(
        target,
        target_root,
        target_items,
        &changes,
        opts,
        &spellings,
        target_pair,
    ) {
        return None;
    }
    let (planned_a, planned_b) = apply_trees(source_side, source_items, target_items, &changes)?;
    let source_empty = if source_side == Side::A {
        planned_a.is_empty()
    } else {
        planned_b.is_empty()
    };
    let source_empty = source_empty && state.dirs.is_none_or(|dirs| dirs.is_empty());
    if let Some(block) = unconfirmed(
        empty_side_block(source_pair, source_empty, state.baseline),
        &state.settings.confirmed,
    ) {
        return Some(Outcome {
            blocked: Some(block),
            baseline: state.baseline.clone(),
            ..Outcome::default()
        });
    }
    let files_a = items_a
        .values()
        .filter(|item| !item.deleted && !item.is_dir)
        .count() as u64;
    let files_b = items_b
        .values()
        .filter(|item| !item.deleted && !item.is_dir)
        .count() as u64;
    let deletes = DeleteCounts::of(&actions);
    if let Some(block) = unconfirmed(
        deletion_block(deletes, files_a, files_b, &opts),
        &state.settings.confirmed,
    ) {
        return Some(Outcome {
            blocked: Some(block),
            baseline: state.baseline.clone(),
            ..Outcome::default()
        });
    }
    // There is only one deleting side in an incremental mirror, but its
    // percentage stop remains independent of a confirmed absolute limit.
    let mut percentage = opts;
    percentage.max_delete = 0;
    if let Some(block) = unconfirmed(
        deletion_block(deletes, files_a, files_b, &percentage),
        &state.settings.confirmed,
    ) {
        return Some(Outcome {
            blocked: Some(block),
            baseline: state.baseline.clone(),
            ..Outcome::default()
        });
    }
    if actions.is_empty() {
        if let Some(cursor) = cursor.as_deref() {
            if store.update_cursor(&pair, Some(cursor)).is_err() {
                return None;
            }
        }
        return Some(Outcome {
            baseline: state.baseline.clone(),
            ..Outcome::default()
        });
    }
    if let Err(error) = retire_index(state) {
        return Some(failure("Sync-Zwischenstand", error));
    }
    let sink = match CheckpointSink::new(endpoints, state.lock, state.key, keys, state.observer) {
        Ok(sink) => sink,
        Err(error) => return Some(failure("Zwischenstand", error)),
    };
    let scope = ApplyScope {
        sink: &sink,
        versions: state.versions,
        spellings: &spellings,
    };
    let mut errors = Vec::new();
    let mut stats = sink.during(|| {
        let copied = super::apply::apply_planned_reporting(
            &planned.upserts,
            &[],
            &planned_a,
            &planned_b,
            endpoints,
            opts,
            &scope,
            &mut errors,
            state.cancel,
        );
        let mut stats = copied.stats;
        // Retired paths are removed only after every upsert has been committed;
        // a failed/deferred copy cannot authorize deleting the user's old path.
        if !state.cancel.load(Ordering::Acquire)
            && errors.is_empty()
            && stats.errors == 0
            && sink.can_delete()
            && !super::ApplySink::should_stop(&sink)
        {
            let deleted = super::apply::apply_planned_reporting(
                &planned.deletes,
                &[],
                &planned_a,
                &planned_b,
                endpoints,
                opts,
                &scope,
                &mut errors,
                state.cancel,
            );
            merge_stats(&mut stats, deleted.stats);
        }
        stats
    });
    let checkpoint = sink.finish();
    let mut omissions = super::SyncOmissions::new(keys.fold_case);
    for (rel, kind) in checkpoint.omitted {
        omissions.record_kind(&rel, kind, kind.reported_by_default());
    }
    if let Some(error) = checkpoint.error {
        errors.push(("Zwischenstand".into(), error));
        stats.errors = stats.errors.saturating_add(1);
    }
    names.applied(&actions, &spellings, &checkpoint.baseline, keys);
    if let Err(error) = super::state_spellings::save(state.key, &names) {
        errors.push(("Pfad-Schreibweisen".into(), error.to_string()));
        stats.errors = stats.errors.saturating_add(1);
    }
    let history = PairHistory {
        replica_a: state.key.replica_a.clone(),
        replica_b: state.key.replica_b.clone(),
        entries_a: super::guards::recorded_entries(&checkpoint.baseline, PairSide::A)
            .saturating_add(checkpoint.dirs.len() as u64),
        entries_b: super::guards::recorded_entries(&checkpoint.baseline, PairSide::B)
            .saturating_add(checkpoint.dirs.len() as u64),
        full_ms: state.history.map_or(0, |history| history.full_ms),
    };
    if let Err(error) = save_history(state.key, &history) {
        errors.push(("Replika-Zustand".into(), error.to_string()));
        stats.errors = stats.errors.saturating_add(1);
    }
    let out = Outcome {
        stats,
        errors,
        baseline: checkpoint.baseline,
        omissions,
        deferred: checkpoint.deferred,
        stopped: checkpoint.stopped,
        ..Outcome::default()
    };
    if out.stats.errors == 0
        && out.omissions.is_empty()
        && out.deferred.is_empty()
        && out.stopped.is_none()
        && !state.cancel.load(Ordering::Acquire)
    {
        let cursor = cursor.or(rec.source_cursor);
        let _ = bootstrap_run(state, &out.baseline, cursor);
    }
    Some(out)
}

fn record_matches(record: &PairRecord, endpoints: SyncEndpoints<'_>, source_side: Side) -> bool {
    record.root_a == endpoints.root_a
        && record.root_b == endpoints.root_b
        && record.source_side == source_side
        && record.bootstrapped
        && record.target_managed
}
fn root_id_matches(backend: &dyn Backend, root: &str, saved: Option<&str>) -> bool {
    saved.is_none_or(|id| backend.change_root_id(root).ok().flatten().as_deref() == Some(id))
}
fn baseline_by_key(base: &Baseline, keys: super::KeyPolicy) -> Baseline {
    base.iter()
        .map(|(rel, entry)| (keys.key(rel).into_owned(), *entry))
        .collect()
}
fn cache_by_key(
    a: &BTreeMap<String, ItemRecord>,
    b: &BTreeMap<String, ItemRecord>,
    keys: super::KeyPolicy,
) -> Option<Baseline> {
    let mut base = Baseline::new();
    for (side, items) in [(PairSide::A, a), (PairSide::B, b)] {
        let mut seen = BTreeSet::new();
        for (rel, item) in items
            .iter()
            .filter(|(_, item)| !item.deleted && !item.is_dir)
        {
            let key = keys.key(rel).into_owned();
            if !seen.insert(key.clone()) {
                return None;
            }
            let entry = base.entry(key).or_default();
            match side {
                PairSide::A => entry.0 = item.sig,
                PairSide::B => entry.1 = item.sig,
            }
        }
    }
    Some(base)
}
fn merge_stats(left: &mut super::BisyncStats, right: super::BisyncStats) {
    left.a_to_b = left.a_to_b.saturating_add(right.a_to_b);
    left.b_to_a = left.b_to_a.saturating_add(right.b_to_a);
    left.deleted = left.deleted.saturating_add(right.deleted);
    left.conflicts = left.conflicts.saturating_add(right.conflicts);
    left.bytes = left.bytes.saturating_add(right.bytes);
    left.errors = left.errors.saturating_add(right.errors);
}
