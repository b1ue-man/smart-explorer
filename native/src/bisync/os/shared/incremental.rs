use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::vfs::{Backend, ChangeKind};

use super::apply::apply_planned_with_results;
use super::incremental_changes::{
    action_plan_for, apply_trees, collect_ids, delete_guard_trips, deleted_item, source_item_after,
    target_item_after, target_touched_drifted,
};
use super::incremental_collect::{
    changes_from_backend, changes_from_source_walk, ChangeCollection,
};
use super::orchestration::Outcome;
use super::persistence::{baseline_path, pair_id_for, save_baseline, versions_dir};
use super::snapshot::WalkFilter;
use super::state_store::{PairRecord, Side, SyncStateStore};
use super::state_validation::baseline_from_items;
use super::types::{Baseline, BisyncOptions, BisyncStats, Conflict, DeletePolicy, Direction};

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

pub(super) fn try_incremental_mirror(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    store_path: Option<&Path>,
) -> Option<Outcome> {
    if opts.dry_run {
        return None;
    }
    if cancel.load(Ordering::Relaxed) {
        return Some(Outcome::default());
    }
    let (source, source_root, source_side) = mirror_source(endpoints, opts)?;
    let SyncEndpoints {
        a,
        root_a,
        b,
        root_b,
    } = endpoints;
    let (target, target_root, target_side) = if source_side == Side::A {
        (b, root_b, Side::B)
    } else {
        (a, root_a, Side::A)
    };
    let pair = pair_id_for(a, root_a, b, root_b);
    let mut store = open_store(store_path).ok()?;
    let rec = store.load_pair(&pair).ok().flatten()?;
    if !record_matches(&rec, root_a, root_b, source_side) {
        return None;
    }
    if !root_id_matches(a, root_a, rec.root_a_id.as_deref())
        || !root_id_matches(b, root_b, rec.root_b_id.as_deref())
    {
        return None;
    }
    // Treat any malformed or over-budget persisted state as an untrusted
    // incremental index. Returning None here selects the bounded full rebuild
    // before any target mutation or cursor advancement.
    let (items_a, items_b) = store.load_pair_items(&pair).ok()?;
    let previous_baseline = baseline_from_items(&items_a, &items_b);
    let (source_items, target_items) = if source_side == Side::A {
        (&items_a, &items_b)
    } else {
        (&items_b, &items_a)
    };
    let collection = if source.supports_changes() {
        changes_from_backend(
            &store,
            &rec,
            source,
            source_root,
            source_side,
            source_items,
            filter,
            cancel,
        )
    } else if source.is_local() {
        changes_from_source_walk(
            source,
            source_root,
            target,
            opts,
            filter,
            source_items,
            cancel,
        )
    } else {
        return None;
    };
    let (changes, new_cursor) = match collection {
        ChangeCollection::Ready {
            changes,
            new_cursor,
        } => (changes, new_cursor),
        ChangeCollection::Rebuild => return None,
        ChangeCollection::Canceled => {
            return Some(Outcome {
                baseline: previous_baseline,
                ..Default::default()
            })
        }
    };
    if cancel.load(Ordering::Relaxed) {
        return Some(Outcome {
            baseline: previous_baseline,
            ..Default::default()
        });
    }
    if changes.is_empty() {
        if let Some(c) = new_cursor.as_deref() {
            if let Err(error) = store.update_cursor(&pair, Some(c)) {
                return Some(Outcome {
                    errors: vec![(
                        pair.clone(),
                        format!("inkrementeller Cursor konnte nicht gespeichert werden: {error}"),
                    )],
                    baseline: previous_baseline,
                    ..Default::default()
                });
            }
        }
        return Some(Outcome {
            baseline: previous_baseline,
            ..Default::default()
        });
    }
    if target_touched_drifted(target, target_root, target_items, &changes, opts) {
        return None;
    }
    let actions = action_plan_for(source_side, &changes);
    let (planned_a, planned_b) = apply_trees(source_side, source_items, target_items, &changes)?;
    if delete_guard_trips(&actions, target_items, opts) {
        return Some(Outcome {
            errors: vec![(
                "abgebrochen".into(),
                "Sicherheitsstopp: inkrementeller Mirror wuerde zu viele Dateien loeschen.".into(),
            )],
            baseline: previous_baseline,
            ..Default::default()
        });
    }
    let mut errors = Vec::new();
    let copy_report = apply_planned_with_results(
        &actions.upserts,
        &planned_a,
        &planned_b,
        a,
        root_a,
        b,
        root_b,
        opts,
        &versions_dir(&pair),
        &mut errors,
        cancel,
    );
    if cancel.load(Ordering::Relaxed) || !errors.is_empty() {
        return Some(Outcome {
            stats: copy_report.stats,
            errors,
            baseline: previous_baseline,
            ..Default::default()
        });
    }
    // Copies publish every final path first. Only after all of them succeed do
    // we remove obsolete paths; swap/cycle destinations were pruned from this
    // phase by action_plan_for and can therefore never be deleted afterward.
    let delete_report = apply_planned_with_results(
        &actions.deletes,
        &planned_a,
        &planned_b,
        a,
        root_a,
        b,
        root_b,
        opts,
        &versions_dir(&pair),
        &mut errors,
        cancel,
    );
    let mut stats = copy_report.stats;
    merge_stats(&mut stats, delete_report.stats);
    if cancel.load(Ordering::Relaxed) || !errors.is_empty() {
        return Some(Outcome {
            stats,
            errors,
            baseline: previous_baseline,
            ..Default::default()
        });
    }
    let mut updates = Vec::new();
    // Tombstone every retired/unmanaged path before writing active upserts.
    // That ordering is important for A<->B rename swaps in one transaction.
    for ch in &changes {
        if let Some(old) = ch.old_rel.as_deref() {
            updates.push(deleted_item(source_side, old, source_items.get(old)));
            updates.push(deleted_item(target_side, old, target_items.get(old)));
        }
        if !ch.managed || ch.kind == ChangeKind::Remove {
            updates.push(deleted_item(
                source_side,
                &ch.rel,
                source_items.get(&ch.rel),
            ));
            updates.push(deleted_item(
                target_side,
                &ch.rel,
                target_items.get(&ch.rel),
            ));
        }
    }
    for ch in changes
        .iter()
        .filter(|change| change.managed && change.kind == ChangeKind::Upsert)
    {
        if cancel.load(Ordering::Relaxed) {
            return Some(Outcome {
                stats,
                baseline: previous_baseline,
                ..Default::default()
            });
        }
        updates.push(source_item_after(source_side, ch));
        let target_update = match target_item_after(target, target_root, target_side, ch) {
            Ok(update) => update,
            Err(error) => {
                return Some(Outcome {
                    stats,
                    errors: vec![(
                        ch.rel.clone(),
                        format!("Zielstatus nach inkrementeller Änderung unklar: {error}"),
                    )],
                    baseline: previous_baseline,
                    ..Default::default()
                });
            }
        };
        updates.push(target_update);
    }
    if cancel.load(Ordering::Relaxed) {
        return Some(Outcome {
            stats,
            baseline: previous_baseline,
            ..Default::default()
        });
    }
    let cursor = new_cursor.as_deref().or(rec.source_cursor.as_deref());
    if let Err(error) = store.save_items_and_cursor(&pair, &updates, cursor) {
        return Some(Outcome {
            stats,
            errors: vec![(
                pair.clone(),
                format!("inkrementeller Stand konnte nicht gespeichert werden: {error}"),
            )],
            baseline: previous_baseline,
            ..Default::default()
        });
    }
    let baseline = match store.load_baseline(&pair) {
        Ok(baseline) => baseline,
        Err(error) => {
            return Some(Outcome {
                stats,
                errors: vec![(
                    pair.clone(),
                    format!("inkrementeller Stand kann nicht gelesen werden: {error}"),
                )],
                baseline: previous_baseline,
                ..Default::default()
            })
        }
    };
    if let Err(error) = save_baseline(&baseline_path(&pair), &baseline) {
        errors.push((
            pair.clone(),
            format!("Legacy-Synchronisierungsstand konnte nicht gespeichert werden: {error}"),
        ));
    }
    Some(Outcome {
        stats,
        conflicts: Vec::<Conflict>::new(),
        errors,
        baseline,
    })
}

fn merge_stats(left: &mut BisyncStats, right: BisyncStats) {
    left.a_to_b = left.a_to_b.saturating_add(right.a_to_b);
    left.b_to_a = left.b_to_a.saturating_add(right.b_to_a);
    left.deleted = left.deleted.saturating_add(right.deleted);
    left.conflicts = left.conflicts.saturating_add(right.conflicts);
    left.bytes = left.bytes.saturating_add(right.bytes);
    left.errors = left.errors.saturating_add(right.errors);
}

pub(super) fn bootstrap_incremental_state(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    baseline: &Baseline,
    source_cursor: Option<String>,
    store_path: Option<&Path>,
) -> rusqlite::Result<()> {
    let Some((_, _, source_side)) = mirror_source(endpoints, opts) else {
        return Ok(());
    };
    let SyncEndpoints {
        a,
        root_a,
        b,
        root_b,
    } = endpoints;
    let pair = pair_id_for(a, root_a, b, root_b);
    let mut store = open_store(store_path)?;
    let rec = PairRecord {
        pair: pair.clone(),
        root_a: root_a.into(),
        root_b: root_b.into(),
        mode: "mirror".into(),
        source_side,
        source_cursor,
        root_a_id: a.change_root_id(root_a).ok().flatten(),
        root_b_id: b.change_root_id(root_b).ok().flatten(),
        bootstrapped: true,
        target_managed: true,
    };
    store.save_pair(&rec)?;
    let ids_a = collect_ids(a, root_a, baseline, Side::A);
    let ids_b = collect_ids(b, root_b, baseline, Side::B);
    store.replace_from_baseline(&pair, baseline, &ids_a, &ids_b)
}

fn open_store(path: Option<&Path>) -> rusqlite::Result<SyncStateStore> {
    path.map_or_else(SyncStateStore::open_default, SyncStateStore::open_at)
}

fn record_matches(rec: &PairRecord, root_a: &str, root_b: &str, source_side: Side) -> bool {
    rec.root_a == root_a
        && rec.root_b == root_b
        && rec.mode == "mirror"
        && rec.source_side == source_side
        && rec.bootstrapped
        && rec.target_managed
}

fn root_id_matches(be: &dyn Backend, root: &str, saved: Option<&str>) -> bool {
    match saved {
        Some(id) => be.change_root_id(root).ok().flatten().as_deref() == Some(id),
        None => true,
    }
}
