use crate::vfs::Backend;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::apply::apply_planned_with_results;
use super::core::{plan, update_baseline};
use super::incremental::{
    bootstrap_incremental_state, mirror_source, try_incremental_mirror, SyncEndpoints,
};
use super::persistence::{
    baseline_path, load_baseline, pair_id_for, prune_versions, save_baseline, versions_dir,
};
use super::snapshot::{
    hash_mode, prev_side, walk_files, walk_files_with_duplicate_files, WalkFilter,
};
use super::types::{
    Action, Baseline, BisyncOptions, BisyncStats, Conflict, DeletePolicy, Direction,
};

/// A read-only comparison of two sync endpoints (the "ls-diff" view): the
/// planned actions + conflicts, with no changes applied. Uses the saved baseline
/// (so it shows what *would* sync, exactly as a real run would decide).
#[derive(Default)]
pub struct Preview {
    pub actions: Vec<Action>,
    pub conflicts: Vec<Conflict>,
    pub a_files: usize,
    pub b_files: usize,
    pub error: Option<String>,
}

pub fn preview(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
) -> Preview {
    let base = match load_baseline(&baseline_path(&pair_id_for(a, root_a, b, root_b))) {
        Ok(base) => base,
        Err(error) => {
            return Preview {
                error: Some(format!(
                    "Synchronisierungsstand kann nicht gelesen werden: {error}"
                )),
                ..Default::default()
            }
        }
    };
    let (mode_a, mode_b) = (hash_mode(a, b, opts.compare), hash_mode(b, a, opts.compare));
    let (prev_a, prev_b) = (prev_side(&base, true), prev_side(&base, false));
    let walk_a = if opts.delete == DeletePolicy::Mirror && opts.direction == Direction::BtoA {
        walk_files_with_duplicate_files
    } else {
        walk_files
    };
    let at = match walk_a(a, root_a, cancel, filter, mode_a, Some(&prev_a)) {
        Ok(t) => t,
        Err(e) => {
            return Preview {
                error: Some(format!("{}: {}", root_a, e)),
                ..Default::default()
            }
        }
    };
    let walk_b = if opts.delete == DeletePolicy::Mirror && opts.direction == Direction::AtoB {
        walk_files_with_duplicate_files
    } else {
        walk_files
    };
    let bt = match walk_b(b, root_b, cancel, filter, mode_b, Some(&prev_b)) {
        Ok(t) => t,
        Err(e) => {
            return Preview {
                error: Some(format!("{}: {}", root_b, e)),
                ..Default::default()
            }
        }
    };
    let (a_files, b_files) = (at.len(), bt.len());
    let (actions, conflicts, _converged) = plan(&at, &bt, &base, opts);
    Preview {
        actions,
        conflicts,
        a_files,
        b_files,
        error: None,
    }
}

// ── high-level orchestration (used by the UI on a worker thread) ─────────────

#[derive(Default)]
pub struct Outcome {
    pub stats: BisyncStats,
    pub conflicts: Vec<Conflict>,
    pub errors: Vec<(String, String)>,
    pub baseline: Baseline,
}

/// One full bisync run: load baseline → walk both → plan → apply → save the
/// new baseline + prune versions. Conflicts are returned (not applied); the
/// updated baseline keeps them flagged until resolved.
pub fn run(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
) -> Outcome {
    let endpoints = SyncEndpoints::new(a, root_a, b, root_b);
    run_inner(endpoints, opts, cancel, filter, None)
}

#[cfg(test)]
pub(super) fn run_with_store_path(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    store_path: &Path,
) -> Outcome {
    run_inner(endpoints, opts, cancel, filter, Some(store_path))
}

fn run_inner(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    store_path: Option<&Path>,
) -> Outcome {
    if let Some(out) = try_incremental_mirror(endpoints, opts, cancel, filter, store_path) {
        return out;
    }

    let SyncEndpoints {
        a,
        root_a,
        b,
        root_b,
    } = endpoints;
    let pre_cursor = mirror_source(endpoints, opts)
        .and_then(|(source, root, _)| source.current_change_cursor(root).ok().flatten());
    let out = run_full(a, root_a, b, root_b, opts, cancel, filter);
    if !opts.dry_run
        && out.errors.is_empty()
        && out.conflicts.is_empty()
        && !cancel.load(Ordering::Relaxed)
    {
        let _ = bootstrap_incremental_state(endpoints, opts, &out.baseline, pre_cursor, store_path);
    }
    out
}

fn run_full(
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
    let walk_a = if opts.delete == DeletePolicy::Mirror && opts.direction == Direction::BtoA {
        walk_files_with_duplicate_files
    } else {
        walk_files
    };
    let at = match walk_a(a, root_a, cancel, filter, mode_a, Some(&prev_a)) {
        Ok(t) => t,
        Err(e) => {
            return Outcome {
                errors: vec![(root_a.into(), e.to_string())],
                ..Default::default()
            }
        }
    };
    let walk_b = if opts.delete == DeletePolicy::Mirror && opts.direction == Direction::AtoB {
        walk_files_with_duplicate_files
    } else {
        walk_files
    };
    let bt = match walk_b(b, root_b, cancel, filter, mode_b, Some(&prev_b)) {
        Ok(t) => t,
        Err(e) => {
            return Outcome {
                errors: vec![(root_b.into(), e.to_string())],
                ..Default::default()
            }
        }
    };
    if cancel.load(Ordering::Relaxed) {
        return Outcome::default();
    }
    let (actions, conflicts, converged) = plan(&at, &bt, &base, opts);

    // Duplicate-name providers need an exact, read-only cleanup plan before
    // the first mutation. Its ID-addressed entries participate in the same
    // all-or-nothing deletion guard as explicit and move-source deletions.
    let (dedupe_backend, dedupe_plan) = if !opts.dry_run && opts.delete == DeletePolicy::Mirror {
        let planned = match opts.direction {
            Direction::AtoB => b
                .plan_dedupe_recursive(root_b, &|rel| at.contains_key(rel))
                .map(|plan| (Some(b), plan)),
            Direction::BtoA => a
                .plan_dedupe_recursive(root_a, &|rel| bt.contains_key(rel))
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
                    ..Default::default()
                }
            }
        }
    } else {
        (None, Vec::new())
    };

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
        .saturating_add(dedupe_plan.len() as u64);
    let total = at.len().max(bt.len()) as u64;
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
    if !opts.dry_run && deletes > 0 && (deletes > abs_limit || deletes > pct_limit) {
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
            ..Default::default()
        };
    }

    let mut errors = Vec::new();
    let deduped = if let Some(backend) = dedupe_backend {
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
                    ..Default::default()
                };
            }
        }
    } else {
        0
    };
    if cancel.load(Ordering::Relaxed) {
        return Outcome {
            stats: BisyncStats {
                deleted: deduped,
                ..Default::default()
            },
            errors,
            baseline: base,
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
        };
    }
    // Re-walk to capture real post-write signatures (e.g. the destination's new
    // mtime), so the baseline doesn't re-detect just-synced files. Skipped on a
    // dry run, and — the common steady-state case — when nothing was actually
    // transferred or deleted: then the on-disk state is unchanged, so the trees
    // we already walked are still current. This avoids a second full metadata
    // walk of a remote (hundreds of Drive round-trips) on every no-op sync.
    let changed = st.a_to_b > 0 || st.b_to_a > 0 || st.deleted > 0;
    let (at2, bt2) = if opts.dry_run || !changed {
        (at, bt)
    } else {
        // Only re-walk a side the run could have modified. A one-way sync without
        // move leaves its SOURCE side untouched, so re-walking it is pure wasted
        // round-trips (decisive when the source is a remote like Drive).
        let a_touched = opts.direction != Direction::AtoB || opts.move_files;
        let b_touched = opts.direction != Direction::BtoA || opts.move_files;
        let at2 = if a_touched {
            match walk_files(a, root_a, cancel, filter, mode_a, Some(&prev_a)) {
                Ok(tree) => tree,
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
                    };
                }
            }
        } else {
            at
        };
        let bt2 = if b_touched {
            match walk_files(b, root_b, cancel, filter, mode_b, Some(&prev_b)) {
                Ok(tree) => tree,
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
                    };
                }
            }
        } else {
            bt
        };
        (at2, bt2)
    };
    let nb = update_baseline(&base, &at2, &bt2, &report.completed, &converged, &conflicts);
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
    }
}
