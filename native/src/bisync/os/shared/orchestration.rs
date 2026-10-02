use crate::vfs::Backend;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::completion::ApplySink;
use super::incremental::{
    bootstrap_incremental_state, invalidate_incremental_state, mirror_source,
    try_incremental_mirror, SyncEndpoints,
};
use super::keys::KeyPolicy;
use super::omissions::SyncOmissions;
use super::orchestration_full::run_full;
use super::pair_lock::pair_lock_id;
use super::persistence::pair_id_for;
use super::run_types::{RunBlock, RunSettings, RunStop, StateKey};
use super::snapshot::WalkFilter;
use super::types::{Baseline, BisyncOptions, BisyncStats, Conflict};

// ── high-level orchestration (used by the UI on a worker thread) ─────────────

#[derive(Default)]
pub struct Outcome {
    pub stats: BisyncStats,
    pub conflicts: Vec<Conflict>,
    pub errors: Vec<(String, String)>,
    pub baseline: Baseline,
    pub omissions: SyncOmissions,
    /// The run stopped before its first change and waits for review (FS3);
    /// neither an error nor a cancel.
    pub blocked: Option<RunBlock>,
    /// The run ended early; what it completed is recorded (FS5).
    pub stopped: Option<RunStop>,
    /// Entries that changed while the run worked on them: nothing was
    /// committed for them and the next run handles them; not errors.
    pub deferred: Vec<(String, String)>,
    /// Another run, conflict resolution or restore of the pair held its
    /// lock; nothing was done.
    pub busy: bool,
    /// The run was canceled.
    pub canceled: bool,
    /// The stored state this run used; conflict resolutions record into it.
    pub state: Option<StateKey>,
    /// This run's id, the folder of its versions.
    pub run_id: Option<String>,
}

/// One run as the background service and the surfaces start it (V3).
pub struct RunRequest<'a> {
    pub a: &'a dyn Backend,
    pub root_a: &'a str,
    pub b: &'a dyn Backend,
    pub root_b: &'a str,
    pub opts: BisyncOptions,
    pub filter: &'a WalkFilter<'a>,
    pub cancel: &'a AtomicBool,
    pub settings: RunSettings,
    /// Also told about every finished action, apply-time omission, deferral
    /// and early stop the moment it happens, from apply's worker threads
    /// (e.g. the watcher, so the run's own writes trigger nothing, B22).
    pub observer: Option<&'a dyn ApplySink>,
}

impl<'a> RunRequest<'a> {
    /// A run without a saved job, as `run` starts it.
    pub fn new(
        a: &'a dyn Backend,
        root_a: &'a str,
        b: &'a dyn Backend,
        root_b: &'a str,
        opts: BisyncOptions,
        filter: &'a WalkFilter<'a>,
        cancel: &'a AtomicBool,
    ) -> Self {
        Self {
            a,
            root_a,
            b,
            root_b,
            opts,
            filter,
            cancel,
            settings: RunSettings::default(),
            observer: None,
        }
    }
}

/// Runs one sync as `request` asks: its owner's state, scan depth, confirmed
/// stops, lock wait and observer. Contract stage: the run of [`run`] on the
/// pair-wide state.
pub fn run_with(request: RunRequest<'_>) -> Outcome {
    let RunRequest {
        a,
        root_a,
        b,
        root_b,
        opts,
        filter,
        cancel,
        settings: _,
        observer: _,
    } = request;
    let mut out = run(a, root_a, b, root_b, opts, cancel, filter);
    out.state = Some(StateKey::legacy(
        &pair_id_for(a, root_a, b, root_b),
        &pair_lock_id(a, root_a, b, root_b),
    ));
    out
}

/// The pair's planning keys: letter case is folded when either side ignores
/// it; the job's ignore patterns follow the same rule (Y153).
pub fn pair_key_policy(a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str) -> KeyPolicy {
    KeyPolicy::for_pair(
        a.case_sensitive_paths(root_a),
        b.case_sensitive_paths(root_b),
    )
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
    let mut out = if let Err(error) = crate::vfs::validate_sync_roots(a, root_a, b, root_b) {
        Outcome {
            errors: vec![("Sync-Pfade".into(), error.to_string())],
            ..Default::default()
        }
    } else {
        run_inner(
            SyncEndpoints::new(a, root_a, b, root_b),
            opts,
            cancel,
            filter,
            None,
        )
    };
    out.canceled = cancel.load(Ordering::Acquire);
    out
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
    if let Err(error) = invalidate_incremental_state(endpoints, opts, store_path) {
        return Outcome {
            errors: vec![(
                "Sync-Index".into(),
                format!("Vollscan kann nicht sicher beginnen: {error}"),
            )],
            ..Default::default()
        };
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
        && out.omissions.is_empty()
        && !cancel.load(Ordering::Relaxed)
    {
        let _ = bootstrap_incremental_state(endpoints, opts, &out.baseline, pre_cursor, store_path);
    }
    out
}
