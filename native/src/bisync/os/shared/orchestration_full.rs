//! The full (non-incremental) run of one pair: walk both sides, plan, check
//! the deletion guard, repair duplicates, apply, observe the result and save
//! the new baseline. Split from `orchestration.rs` (file size); the entry
//! points stay there.
use crate::vfs::Backend;
use std::sync::atomic::{AtomicBool, Ordering};

use super::apply::apply_planned_with_results;
use super::core::update_baseline;
use super::incremental::SyncEndpoints;
use super::orchestration::Outcome;
use super::persistence::{
    baseline_path, load_baseline, pair_id_for, prune_versions, save_baseline, versions_dir,
};
use super::snapshot::{hash_mode, prev_side, walk_snapshot, WalkFilter};
use super::snapshot_pair::read_pair;
use super::types::{Action, BisyncOptions, BisyncStats, DeletePolicy, Direction};

pub(super) fn run_full(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
) -> Outcome {
    let pair = pair_id_for(a, root_a, b, root_b);
    let bpath = baseline_path(&pair);
    let vdir = versions_dir(&pair);
    let base = match load_baseline(&bpath) {
        Ok(base) => base,
        Err(error) => {
            return Outcome {
                errors: vec![(
                    bpath.to_string_lossy().into_owned(),
                    format!("Synchronisierungsstand kann nicht gelesen werden: {error}"),
                )],
                ..Default::default()
            }
        }
    };
    // Per-side hashing: each side uses a content hash when it's free (native) or
    // cheap (a local read to match the other side's free native hash), so any
    // compare mode skips files whose mtime differs but content matches — without
    // ever downloading a hash-less remote. `prev_*` reuses last run's hashes.
    let (mode_a, mode_b) = (hash_mode(a, b, opts.compare), hash_mode(b, a, opts.compare));
    let (prev_a, prev_b) = (prev_side(&base, true), prev_side(&base, false));
    let snapshot = match read_pair(
        SyncEndpoints::new(a, root_a, b, root_b),
        opts,
        cancel,
        filter,
        &base,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Outcome {
                errors: vec![error],
                baseline: base,
                ..Default::default()
            }
        }
    };
    let (actions, conflicts, converged) = snapshot.plan(&base, opts);
    let physical_a = snapshot.a.len() as u64
        + snapshot
            .repairs
            .iter()
            .filter_map(|c| c.duplicates.as_ref())
            .map(|d| d.a.len().saturating_sub(1) as u64)
            .sum::<u64>()
        + snapshot
            .conflicts
            .iter()
            .filter_map(|c| c.duplicates.as_ref())
            .map(|d| d.a.len() as u64)
            .sum::<u64>();
    let physical_b = snapshot.b.len() as u64
        + snapshot
            .repairs
            .iter()
            .filter_map(|c| c.duplicates.as_ref())
            .map(|d| d.b.len().saturating_sub(1) as u64)
            .sum::<u64>()
        + snapshot
            .conflicts
            .iter()
            .filter_map(|c| c.duplicates.as_ref())
            .map(|d| d.b.len() as u64)
            .sum::<u64>();
    let repairs = snapshot.repairs;
    let duplicate_removals: u64 = repairs
        .iter()
        .filter_map(|c| c.duplicates.as_ref())
        .map(|d| d.redundant_count())
        .sum();
    let (at, bt) = (snapshot.a, snapshot.b);
    let mut omissions = snapshot.omissions;
    if cancel.load(Ordering::Relaxed) {
        return Outcome {
            baseline: base,
            omissions,
            ..Default::default()
        };
    }

    // Duplicate-name providers need an exact, read-only cleanup plan before
    // the first mutation. Its ID-addressed entries participate in the same
    // all-or-nothing deletion guard as explicit and move-source deletions.
    let (dedupe_backend, mut dedupe_plan) = if !opts.dry_run && opts.delete == DeletePolicy::Mirror
    {
        let planned = match opts.direction {
            Direction::AtoB => b
                .plan_dedupe_recursive(root_b, &|rel| {
                    at.contains_key(rel) || omissions.protects(rel)
                })
                .map(|plan| (Some(b), plan)),
            Direction::BtoA => a
                .plan_dedupe_recursive(root_a, &|rel| {
                    bt.contains_key(rel) || omissions.protects(rel)
                })
                .map(|plan| (Some(a), plan)),
            Direction::Both => Ok((None, Vec::new())),
        };
        match planned {
            Ok(result) => result,
            Err(error) => {
                return Outcome {
                    errors: vec![(
                        "Duplikatprüfung".into(),
                        format!("Duplikate konnten nicht sicher vorgeprüft werden: {error}"),
                    )],
                    baseline: base,
                    omissions,
                    ..Default::default()
                }
            }
        }
    } else {
        (None, Vec::new())
    };
    let dedupe_root = if opts.direction == Direction::AtoB {
        root_b
    } else {
        root_a
    };
    dedupe_plan
        .retain(|entry| !omissions.protects(&super::paths::rel_of(&entry.path, dedupe_root)));

    // Delete-safety guard: refuse to apply if the plan would remove more files
    // than the configured limit (protects against a vanished/remounted side
    // looking like a mass deletion). Aborts the whole run — nothing is touched.
    let explicit_deletes = actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                Action::DeleteA(_)
                    | Action::DeleteB(_)
                    | Action::FinalizeMoveAtoB(_)
                    | Action::FinalizeMoveBtoA(_)
            )
        })
        .count() as u64;
    let move_deletes = if opts.move_files && opts.direction != Direction::Both {
        actions
            .iter()
            .filter(|action| matches!(action, Action::CopyAtoB(_) | Action::CopyBtoA(_)))
            .count() as u64
    } else {
        0
    };
    let deletes = explicit_deletes
        .saturating_add(move_deletes)
        .saturating_add(duplicate_removals)
        .saturating_add(dedupe_plan.len() as u64);
    let total = physical_a.max(physical_b);
    let pct_limit = if opts.max_delete_pct > 0 {
        total * opts.max_delete_pct as u64 / 100
    } else {
        u64::MAX
    };
    let abs_limit = if opts.max_delete > 0 {
        opts.max_delete
    } else {
        u64::MAX
    };
    // The percentage stop counts from `max_delete_min` deletions on, so a
    // small folder can still be cleaned up (0: the percentage alone).
    let pct_tripped = deletes >= opts.max_delete_min && deletes > pct_limit;
    if !opts.dry_run && deletes > 0 && (deletes > abs_limit || pct_tripped) {
        return Outcome {
            errors: vec![(
                "abgebrochen".into(),
                format!(
                    "Sicherheitsstopp: {} Löschungen überschreiten das Limit \
                     (max {} Dateien / {}%). Nichts wurde geändert.",
                    deletes, opts.max_delete, opts.max_delete_pct
                ),
            )],
            baseline: base,
            omissions,
            ..Default::default()
        };
    }

    let mut errors = Vec::new();
    let mut deduped = if let Some(backend) = dedupe_backend {
        match backend.apply_dedupe_plan(&dedupe_plan) {
            Ok(count) => count as u64,
            Err(error) => {
                errors.push((
                    "dedupe".into(),
                    format!("Vorgeprüfte Duplikatbereinigung fehlgeschlagen: {error}"),
                ));
                return Outcome {
                    errors,
                    baseline: base,
                    omissions,
                    ..Default::default()
                };
            }
        }
    } else {
        0
    };
    if !opts.dry_run {
        for repair in &repairs {
            let id = repair
                .duplicates
                .as_ref()
                .and_then(|d| d.common_choice())
                .and_then(|(a, _)| a.id.as_deref());
            if let Err(error) = super::duplicate_apply::resolve(
                SyncEndpoints::new(a, root_a, b, root_b),
                repair,
                true,
                id,
                &vdir,
                cancel,
                opts.bwlimit_bps,
                |_| {},
            ) {
                errors.push((repair.rel.clone(), format!("Duplikatbereinigung: {error}")));
                return Outcome {
                    errors,
                    baseline: base,
                    omissions,
                    conflicts,
                    ..Default::default()
                };
            }
            deduped += repair
                .duplicates
                .as_ref()
                .map_or(0, |d| d.redundant_count());
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Outcome {
            stats: BisyncStats {
                deleted: deduped,
                ..Default::default()
            },
            errors,
            baseline: base,
            omissions,
            ..Default::default()
        };
    }
    let report = apply_planned_with_results(
        &actions,
        &at,
        &bt,
        a,
        root_a,
        b,
        root_b,
        opts,
        &vdir,
        &mut errors,
        cancel,
    );
    let mut st = report.stats;
    st.deleted = st.deleted.saturating_add(deduped);
    // Stop pressed: `apply` broke out between files. Don't dedupe or re-walk (a
    // cancelled walk returns a PARTIAL tree, which would corrupt the baseline) —
    // return what completed, leaving the old baseline untouched so the next run
    // re-detects cleanly.
    if cancel.load(Ordering::Relaxed) {
        return Outcome {
            stats: st,
            conflicts,
            errors,
            baseline: base,
            omissions,
            ..Default::default()
        };
    }
    // A failed copy/source action can leave a retryable partial transition.
    // Do not perform any additional destructive mirror cleanup in that state.
    if !errors.is_empty() {
        return Outcome {
            stats: st,
            conflicts,
            errors,
            baseline: base,
            omissions,
            ..Default::default()
        };
    }
    // Re-walk to capture real post-write signatures (e.g. the destination's new
    // mtime), so the baseline doesn't re-detect just-synced files. Skipped on a
    // dry run, and — the common steady-state case — when nothing was actually
    // transferred or deleted: then the on-disk state is unchanged, so the trees
    // we already walked are still current. This avoids a second full metadata
    // walk of a remote (hundreds of Drive round-trips) on every no-op sync.
    let changed = st.a_to_b > 0 || st.b_to_a > 0 || st.deleted > 0;
    let fold_case = !a.case_sensitive_paths(root_a) || !b.case_sensitive_paths(root_b);
    let (mut at2, mut bt2) = if opts.dry_run || !changed {
        (at, bt)
    } else {
        // Only re-walk a side the run could have modified. A one-way sync without
        // move leaves its SOURCE side untouched, so re-walking it is pure wasted
        // round-trips (decisive when the source is a remote like Drive).
        let a_touched = opts.direction != Direction::AtoB || opts.move_files || !repairs.is_empty();
        let b_touched = opts.direction != Direction::BtoA || opts.move_files || !repairs.is_empty();
        let at2 = if a_touched {
            match walk_snapshot(
                a,
                root_a,
                cancel,
                filter,
                mode_a,
                Some(&prev_a),
                false,
                fold_case,
            )
            .and_then(|s| super::duplicate_plan::check_post_scan(s, &conflicts))
            {
                Ok(snapshot) => {
                    omissions.extend(snapshot.omissions);
                    snapshot.tree
                }
                Err(error) => {
                    errors.push((
                        root_a.into(),
                        format!("Kontrollscan nach Änderungen fehlgeschlagen: {error}"),
                    ));
                    return Outcome {
                        stats: st,
                        conflicts,
                        errors,
                        baseline: base,
                        omissions,
                        ..Default::default()
                    };
                }
            }
        } else {
            at
        };
        let bt2 = if b_touched {
            match walk_snapshot(
                b,
                root_b,
                cancel,
                filter,
                mode_b,
                Some(&prev_b),
                false,
                fold_case,
            )
            .and_then(|s| super::duplicate_plan::check_post_scan(s, &conflicts))
            {
                Ok(snapshot) => {
                    omissions.extend(snapshot.omissions);
                    snapshot.tree
                }
                Err(error) => {
                    errors.push((
                        root_b.into(),
                        format!("Kontrollscan nach Änderungen fehlgeschlagen: {error}"),
                    ));
                    return Outcome {
                        stats: st,
                        conflicts,
                        errors,
                        baseline: base,
                        omissions,
                        ..Default::default()
                    };
                }
            }
        } else {
            bt
        };
        (at2, bt2)
    };
    omissions.exclude_tree(&mut at2);
    omissions.exclude_tree(&mut bt2);
    let planning_base = omissions.planning_baseline(&base);
    let mut nb = update_baseline(
        &planning_base,
        &at2,
        &bt2,
        &report.completed,
        &converged,
        &conflicts,
    );
    omissions.preserve_baseline(&base, &mut nb);
    if !opts.dry_run {
        if let Err(error) = save_baseline(&bpath, &nb) {
            errors.push((
                bpath.to_string_lossy().into_owned(),
                format!("Synchronisierungsstand konnte nicht gespeichert werden: {error}"),
            ));
            return Outcome {
                stats: st,
                conflicts,
                errors,
                baseline: base,
                omissions,
                ..Default::default()
            };
        }
        if let Err(error) = prune_versions(&vdir, &opts.versioning) {
            errors.push((
                vdir.to_string_lossy().into_owned(),
                format!(
                    "Wiederherstellungsversionen konnten nicht sicher bereinigt werden: {error}"
                ),
            ));
        }
    }
    Outcome {
        stats: st,
        conflicts,
        errors,
        baseline: nb,
        omissions,
        ..Default::default()
    }
}
