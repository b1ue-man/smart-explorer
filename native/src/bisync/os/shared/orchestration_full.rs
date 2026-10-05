//! Complete paired observation, safety guards and reporting apply. Baseline
//! changes come only from observed convergence or authoritative apply results.
use std::sync::atomic::Ordering;

use super::checkpoint::ApplyScope;
use super::checkpoint_journal::Frame;
use super::checkpoint_run::CheckpointSink;
use super::guards::{unconfirmed, DeleteCounts};
use super::omissions::OmissionKind;
use super::orchestration::{failure, Outcome, RunState};
use super::orchestration_plan::{blocked, context, prepare_spelled};
use super::run_types::RunBlock;
use super::snapshot_pair::read_pair;
use super::state_metadata::{now_ms, save_history, PairHistory};
use super::types::{Action, DeletePolicy, Direction, PairSide};

pub(super) fn run_full_locked(state: &RunState<'_>) -> Outcome {
    let endpoints = state.endpoints;
    let opts = state.opts;
    let keys = super::orchestration_plan::keys(endpoints);
    let mut names = match super::state_spellings::load(state.key, keys) {
        Ok(names) => names,
        Err(error) => {
            return Outcome {
                baseline: state.baseline.clone(),
                ..failure("Pfad-Schreibweisen", error)
            }
        }
    };
    if let Err(error) = super::incremental::retire_index(state) {
        return failure("Sync-Zwischenstand", error);
    }
    let cursor = super::incremental::mirror_source(endpoints, opts)
        .and_then(|(source, root, _)| source.current_change_cursor(root).ok().flatten());
    let mut snapshot = match read_pair(endpoints, opts, state.cancel, state.filter, state.baseline)
    {
        Ok(snapshot) => snapshot,
        Err((path, error)) => {
            return Outcome {
                baseline: state.baseline.clone(),
                ..failure(&path, error)
            }
        }
    };
    let empty_a = snapshot.a.is_empty();
    let empty_b = snapshot.b.is_empty();
    let observed_counts = [snapshot.a.entry_count(), snapshot.b.entry_count()];
    if let Err(error) = super::orchestration_plan::protect_pending_spelled(
        state.lock,
        state.key,
        endpoints,
        keys,
        &names.aliases,
        &mut snapshot,
    ) {
        return Outcome {
            baseline: state.baseline.clone(),
            ..failure("Merge-Wiederanlauf", error)
        };
    }
    // Rotation has an independent baseline, but an empty newly observed
    // volume still requires explicit acceptance before mirror deletion.
    if let Some(history) = state.history {
        for (side, empty, previous) in [
            (PairSide::A, empty_a, history.entries_a),
            (PairSide::B, empty_b, history.entries_b),
        ] {
            let block = (empty && previous > 0).then_some(RunBlock::SideEmpty { side, previous });
            if let Some(block) = unconfirmed(block, &state.settings.confirmed) {
                return Outcome {
                    blocked: Some(block),
                    baseline: state.baseline.clone(),
                    ..Outcome::default()
                };
            }
        }
    }
    // Conflicting duplicate groups never appear absent to the planner.
    let mut base = state.baseline.clone();
    for conflict in &snapshot.conflicts {
        base.remove(&conflict.rel);
        snapshot.a.omissions.record_kind(
            &conflict.rel,
            OmissionKind::NameImpossibleOnTarget,
            false,
        );
    }
    let ctx = context(endpoints, opts, state.dirs);
    let mut plan = prepare_spelled(
        endpoints,
        &mut snapshot.a,
        &mut snapshot.b,
        &base,
        &ctx,
        &names.aliases,
        state.cancel,
    );
    names.observe(PairSide::A, &snapshot.a, keys);
    names.observe(PairSide::B, &snapshot.b, keys);
    plan.conflicts.extend(snapshot.conflicts);
    let repair_keys: std::collections::BTreeSet<_> = snapshot
        .repairs
        .iter()
        .map(|repair| keys.key(&repair.rel).into_owned())
        .collect();
    let mut deletes = DeleteCounts::of(&plan.actions);
    for repair in &snapshot.repairs {
        if let Some(group) = &repair.duplicates {
            deletes.add(PairSide::A, group.a.len().saturating_sub(1) as u64);
            deletes.add(PairSide::B, group.b.len().saturating_sub(1) as u64);
            plan.files_a = plan
                .files_a
                .saturating_add(group.a.len().saturating_sub(1) as u64);
            plan.files_b = plan
                .files_b
                .saturating_add(group.b.len().saturating_sub(1) as u64);
        }
    }
    let mut errors = Vec::new();
    let mut error_count = 0u64;
    let (dedupe_backend, dedupe_root, dedupe_side) = match (opts.delete, opts.direction) {
        (DeletePolicy::Mirror, Direction::AtoB) if !opts.move_files => {
            (Some(endpoints.b), endpoints.root_b, PairSide::B)
        }
        (DeletePolicy::Mirror, Direction::BtoA) if !opts.move_files => {
            (Some(endpoints.a), endpoints.root_a, PairSide::A)
        }
        _ => (None, "", PairSide::B),
    };
    let mut dedupe = Vec::new();
    if let Some(backend) = dedupe_backend {
        let source = if dedupe_side == PairSide::B {
            &snapshot.a.tree
        } else {
            &snapshot.b.tree
        };
        let source_keys: std::collections::BTreeSet<_> = source
            .keys()
            .map(|rel| names.aliases.key(rel, dedupe_side.other(), keys))
            .collect();
        match backend.plan_dedupe_recursive(dedupe_root, &|rel| {
            source_keys.contains(&names.aliases.key(rel, dedupe_side, keys))
                || plan.omissions.protects(rel)
        }) {
            Ok(planned) => dedupe = planned,
            Err(error) => {
                // Failed enumeration cannot authorize duplicate deletions.
                // Regular guarded actions remain independent of this optional
                // cleanup plan and may still proceed.
                errors.push(("Duplikatprüfung".into(), error.to_string()));
                error_count += 1;
            }
        }
        dedupe.retain(|entry| {
            let rel = super::paths::rel_of(&entry.path, dedupe_root);
            !plan.omissions.protects(&rel) && !repair_keys.contains(keys.key(&rel).as_ref())
        });
        let orphans: std::collections::BTreeSet<_> = dedupe
            .iter()
            .map(|entry| {
                names.aliases.key(
                    &super::paths::rel_of(&entry.path, dedupe_root),
                    dedupe_side,
                    keys,
                )
            })
            .filter(|key| !source_keys.contains(key))
            .collect();
        // The primary orphan is already an ordinary Delete action. Count
        // each physical ID only once, and include its extra IDs in the divisor.
        let extra = (dedupe.len() as u64).saturating_sub(orphans.len() as u64);
        deletes.add(dedupe_side, extra);
        match dedupe_side {
            PairSide::A => plan.files_a = plan.files_a.saturating_add(extra),
            PairSide::B => plan.files_b = plan.files_b.saturating_add(extra),
        }
    }
    if let Some(block) = blocked(
        &plan,
        &snapshot.a,
        &snapshot.b,
        &base,
        &opts,
        state.settings,
        deletes,
    ) {
        return Outcome {
            blocked: Some(block),
            baseline: state.baseline.clone(),
            omissions: plan.omissions,
            conflicts: plan.conflicts,
            ..Outcome::default()
        };
    }
    if state.cancel.load(Ordering::Acquire) {
        return Outcome {
            baseline: state.baseline.clone(),
            omissions: plan.omissions,
            ..Outcome::default()
        };
    }
    if opts.dry_run {
        let sink = super::checkpoint::CollectingSink::default();
        let scope = ApplyScope {
            sink: &sink,
            versions: state.versions,
            spellings: &plan.spellings,
        };
        let report = super::apply::apply_planned_reporting(
            &plan.actions,
            &plan.dirs,
            &snapshot.a.tree,
            &snapshot.b.tree,
            endpoints,
            opts,
            &scope,
            &mut errors,
            state.cancel,
        );
        return Outcome {
            stats: report.stats,
            baseline: state.baseline.clone(),
            errors,
            conflicts: plan.conflicts,
            omissions: plan.omissions,
            ..Outcome::default()
        };
    }
    let sink = match CheckpointSink::new(endpoints, state.lock, state.key, ctx.keys, state.observer)
    {
        Ok(sink) => {
            sink.with_observations([&snapshot.a, &snapshot.b], &plan.spellings, observed_counts)
        }
        Err(error) => return failure("Zwischenstand", error),
    };
    // Duplicate repairs own their exact rels. Their old basis is not replaced
    // by a tentative convergence record before the repair actually succeeded.
    plan.records
        .retain(|(rel, _)| !repair_keys.contains(keys.key(rel).as_ref()));
    plan.forget
        .retain(|rel| !repair_keys.contains(keys.key(rel).as_ref()));
    let present_dirs: std::collections::BTreeSet<_> = snapshot
        .a
        .dirs
        .iter()
        .map(|rel| names.aliases.key(rel, PairSide::A, keys))
        .chain(
            snapshot
                .b
                .dirs
                .iter()
                .map(|rel| names.aliases.key(rel, PairSide::B, keys)),
        )
        .collect();
    let dirs_remove = state
        .dirs
        .into_iter()
        .flatten()
        .filter(|dir| {
            !present_dirs.contains(keys.key(dir).as_ref()) && !plan.omissions.protects(dir)
        })
        .cloned()
        .collect();
    if let Err(error) = sink.planned(Frame {
        records: std::mem::take(&mut plan.records),
        forget: std::mem::take(&mut plan.forget),
        dirs_add: std::mem::take(&mut plan.dirs_in_sync).into_iter().collect(),
        dirs_remove,
        ..Frame::default()
    }) {
        return failure("Zwischenstand", error);
    }
    let mut deduped = 0u64;
    let dedupe_scope = ApplyScope {
        sink: &sink,
        versions: state.versions,
        spellings: &plan.spellings,
    };
    if !dedupe.is_empty() {
        let report = sink.during(|| {
            super::apply::apply_dedupe_reporting(
                &dedupe,
                dedupe_side,
                &snapshot.a.tree,
                &snapshot.b.tree,
                endpoints,
                opts,
                &dedupe_scope,
                &mut errors,
                state.cancel,
            )
        });
        deduped = report.stats.deleted;
        error_count = error_count.saturating_add(report.stats.errors);
        plan.omissions.extend(sink.protected_paths());
    }
    for repair in &snapshot.repairs {
        if state.cancel.load(Ordering::Acquire) || super::ApplySink::should_stop(&sink) {
            break;
        }
        if plan.omissions.protects(&repair.rel) {
            continue;
        }
        let id = repair
            .duplicates
            .as_ref()
            .and_then(|group| group.common_choice())
            .and_then(|(a, _)| a.id.as_deref());
        match super::duplicate_apply::resolve_scoped(
            endpoints,
            repair,
            true,
            id,
            opts,
            &dedupe_scope,
            state.cancel,
            opts.bwlimit_bps,
            |_| {},
        ) {
            Ok(entry) => {
                deduped = deduped.saturating_add(
                    repair
                        .duplicates
                        .as_ref()
                        .map_or(0, |group| group.redundant_count()),
                );
                if let Err(error) = sink.planned(Frame {
                    records: vec![(repair.rel.clone(), entry)],
                    ..Frame::default()
                }) {
                    errors.push((repair.rel.clone(), error.to_string()));
                    error_count += 1;
                    break;
                }
            }
            Err(error) => {
                plan.omissions
                    .record_kind(&repair.rel, OmissionKind::Unreadable, true);
                errors.push((repair.rel.clone(), format!("Duplikatbereinigung: {error}")));
                error_count += 1;
            }
        }
    }
    plan.omissions.extend(sink.protected_paths());
    plan.actions
        .retain(|action| !plan.omissions.protects(super::core::action_rel(action)));
    plan.actions.retain(|action| match action {
        Action::DeleteA(rel) => !sink.completed_deletion(rel, PairSide::A),
        Action::DeleteB(rel) => !sink.completed_deletion(rel, PairSide::B),
        _ => true,
    });
    plan.dirs
        .retain(|action| !plan.omissions.protects(action.rel()));
    let scope = ApplyScope {
        sink: &sink,
        versions: state.versions,
        spellings: &plan.spellings,
    };
    let report = sink.during(|| {
        super::apply::apply_planned_reporting(
            &plan.actions,
            &plan.dirs,
            &snapshot.a.tree,
            &snapshot.b.tree,
            endpoints,
            opts,
            &scope,
            &mut errors,
            state.cancel,
        )
    });
    let checkpoint = sink.finish();
    names.applied(
        &plan.actions,
        &plan.spellings,
        state.baseline,
        &checkpoint.baseline,
        keys,
    );
    for (rel, kind) in checkpoint.omitted {
        plan.omissions
            .record_kind(&rel, kind, kind.reported_by_default());
    }
    let mut stats = report.stats;
    stats.deleted = stats.deleted.saturating_add(deduped);
    stats.errors = stats.errors.saturating_add(error_count);
    stats.conflicts = plan.conflicts.len() as u64;
    if let Some(error) = checkpoint.error {
        errors.push(("Zwischenstand".into(), error));
        stats.errors = stats.errors.saturating_add(1);
    }
    if let Err(error) = super::state_spellings::save(state.key, &names) {
        errors.push(("Pfad-Schreibweisen".into(), error.to_string()));
        stats.errors = stats.errors.saturating_add(1);
    }
    let counts = checkpoint.counts.unwrap_or(observed_counts);
    let history = PairHistory {
        replica_a: state.key.replica_a.clone(),
        replica_b: state.key.replica_b.clone(),
        entries_a: counts[0],
        entries_b: counts[1],
        full_ms: now_ms(),
    };
    if let Err(error) = save_history(state.key, &history) {
        errors.push(("Replika-Zustand".into(), error.to_string()));
        stats.errors = stats.errors.saturating_add(1);
    }
    let out = Outcome {
        stats,
        conflicts: plan.conflicts,
        errors,
        baseline: checkpoint.baseline,
        omissions: plan.omissions,
        deferred: checkpoint.deferred,
        stopped: checkpoint.stopped,
        ..Outcome::default()
    };
    if out.stats.errors == 0
        && out.conflicts.is_empty()
        && out.omissions.is_empty()
        && out.deferred.is_empty()
        && out.stopped.is_none()
        && !state.cancel.load(Ordering::Acquire)
    {
        // Cache failures never invalidate completed file work. Its dirty marker
        // keeps selecting the full planner until a bootstrap succeeds.
        let _ = super::incremental::bootstrap_run(state, &out.baseline, cursor);
    }
    out
}
