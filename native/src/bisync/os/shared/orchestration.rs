use crate::vfs::Backend;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::apply::apply_with_results;
use super::core::{plan, update_baseline};
use super::incremental::{bootstrap_incremental_state, mirror_source, try_incremental_mirror};
use super::persistence::{
    baseline_path, load_baseline, pair_id, prune_versions, save_baseline, versions_dir,
};
use super::snapshot::{hash_mode, prev_side, walk_files, WalkFilter};
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
    let base = load_baseline(&baseline_path(&pair_id(root_a, root_b)));
    let (mode_a, mode_b) = (hash_mode(a, b, opts.compare), hash_mode(b, a, opts.compare));
    let (prev_a, prev_b) = (prev_side(&base, true), prev_side(&base, false));
    let at = match walk_files(a, root_a, cancel, filter, mode_a, Some(&prev_a)) {
        Ok(t) => t,
        Err(e) => {
            return Preview {
                error: Some(format!("{}: {}", root_a, e)),
                ..Default::default()
            }
        }
    };
    let bt = match walk_files(b, root_b, cancel, filter, mode_b, Some(&prev_b)) {
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
    run_inner(a, root_a, b, root_b, opts, cancel, filter, None)
}

#[cfg(test)]
pub(super) fn run_with_store_path(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    store_path: &Path,
) -> Outcome {
    run_inner(a, root_a, b, root_b, opts, cancel, filter, Some(store_path))
}

fn run_inner(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    store_path: Option<&Path>,
) -> Outcome {
    if let Some(out) =
        try_incremental_mirror(a, root_a, b, root_b, opts, cancel, filter, store_path)
    {
        return out;
    }

    let pre_cursor = mirror_source(a, root_a, b, root_b, opts)
        .and_then(|(source, root, _)| source.current_change_cursor(root).ok().flatten());
    let out = run_full(a, root_a, b, root_b, opts, cancel, filter);
    if !opts.dry_run
        && out.errors.is_empty()
        && out.conflicts.is_empty()
        && !cancel.load(Ordering::Relaxed)
    {
        let _ = bootstrap_incremental_state(
            a,
            root_a,
            b,
            root_b,
            opts,
            &out.baseline,
            pre_cursor,
            store_path,
        );
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
    let pair = pair_id(root_a, root_b);
    let bpath = baseline_path(&pair);
    let vdir = versions_dir(&pair);
    let base = load_baseline(&bpath);
    // Per-side hashing: each side uses a content hash when it's free (native) or
    // cheap (a local read to match the other side's free native hash), so any
    // compare mode skips files whose mtime differs but content matches — without
    // ever downloading a hash-less remote. `prev_*` reuses last run's hashes.
    let (mode_a, mode_b) = (hash_mode(a, b, opts.compare), hash_mode(b, a, opts.compare));
    let (prev_a, prev_b) = (prev_side(&base, true), prev_side(&base, false));
    let at = match walk_files(a, root_a, cancel, filter, mode_a, Some(&prev_a)) {
        Ok(t) => t,
        Err(e) => {
            return Outcome {
                errors: vec![(root_a.into(), e.to_string())],
                ..Default::default()
            }
        }
    };
    let bt = match walk_files(b, root_b, cancel, filter, mode_b, Some(&prev_b)) {
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

    // Delete-safety guard: refuse to apply if the plan would remove more files
    // than the configured limit (protects against a vanished/remounted side
    // looking like a mass deletion). Aborts the whole run — nothing is touched.
    let deletes = actions
        .iter()
        .filter(|a| matches!(a, Action::DeleteA(_) | Action::DeleteB(_)))
        .count() as u64;
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
    let report = apply_with_results(
        &actions,
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
    // Mirror = exact replica: remove duplicate same-name files the destination
    // backend may hold (e.g. Google Drive) so only the correct one remains. This
    // runs before the re-walk so the baseline reflects the deduped state.
    if !opts.dry_run && opts.delete == DeletePolicy::Mirror {
        let dedup = match opts.direction {
            // Keep only names present on the source side (the mirror's truth);
            // an orphaned duplicate name is removed entirely.
            Direction::AtoB => b.dedupe_recursive(root_b, &|rel| at.contains_key(rel)).ok(),
            Direction::BtoA => a.dedupe_recursive(root_a, &|rel| bt.contains_key(rel)).ok(),
            Direction::Both => None,
        };
        st.deleted += dedup.unwrap_or(0) as u64;
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
            walk_files(a, root_a, cancel, filter, mode_a, Some(&prev_a)).unwrap_or(at)
        } else {
            at
        };
        let bt2 = if b_touched {
            walk_files(b, root_b, cancel, filter, mode_b, Some(&prev_b)).unwrap_or(bt)
        } else {
            bt
        };
        (at2, bt2)
    };
    let nb = update_baseline(&base, &at2, &bt2, &report.completed, &converged, &conflicts);
    if !opts.dry_run {
        let _ = save_baseline(&bpath, &nb);
        prune_versions(&vdir, &opts.versioning);
    }
    Outcome {
        stats: st,
        conflicts,
        errors,
        baseline: nb,
    }
}
