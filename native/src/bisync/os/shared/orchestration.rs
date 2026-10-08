use crate::vfs::Backend;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::completion::ApplySink;
use super::incremental::SyncEndpoints;
use super::keys::KeyPolicy;
use super::omissions::SyncOmissions;
use super::pair_lock::pair_lock_id;
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

/// The pair's planning keys, shared by planning and job ignore patterns.
pub fn pair_key_policy(a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str) -> KeyPolicy {
    KeyPolicy::for_pair(
        a.case_sensitive_paths(root_a),
        b.case_sensitive_paths(root_b),
    )
}

pub fn run_with(request: RunRequest<'_>) -> Outcome {
    run_at(request, None)
}

pub fn run(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
) -> Outcome {
    run_with(RunRequest::new(a, root_a, b, root_b, opts, filter, cancel))
}

#[cfg(test)]
pub(super) fn run_with_store_path(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    store_path: &Path,
) -> Outcome {
    run_at(
        RunRequest::new(
            endpoints.a,
            endpoints.root_a,
            endpoints.b,
            endpoints.root_b,
            opts,
            filter,
            cancel,
        ),
        Some(store_path),
    )
}

pub(super) struct RunState<'a> {
    pub endpoints: SyncEndpoints<'a>,
    pub opts: BisyncOptions,
    pub settings: &'a RunSettings,
    pub cancel: &'a AtomicBool,
    pub filter: &'a WalkFilter<'a>,
    pub observer: Option<&'a dyn ApplySink>,
    pub lock: &'a super::PairLock,
    pub key: &'a StateKey,
    pub history: Option<&'a super::state_metadata::PairHistory>,
    pub baseline: &'a Baseline,
    pub dirs: Option<&'a super::DirSet>,
    pub versions: &'a super::versions::RunVersions,
    pub store_path: Option<&'a Path>,
}

/// Runs with the saved job's live log (`run_log`): the walk, the comparisons
/// and every apply event of this run are written to the job's log file.
fn run_at(request: RunRequest<'_>, store_path: Option<&Path>) -> Outcome {
    let log = match &request.settings.owner {
        super::StateOwner::Job(id) => super::run_log::open_job_log(id),
        super::StateOwner::AdHoc => None,
    };
    let Some(log) = log else {
        return run_at_inner(request, store_path);
    };
    let _current = super::run_log::enter(Some(log.clone()));
    super::run_log_lines::start_line(&log, &request);
    let logging = super::run_log::LoggingSink {
        log: log.clone(),
        inner: request.observer,
    };
    let started = std::time::Instant::now();
    let out = run_at_inner(
        RunRequest {
            observer: Some(&logging),
            ..request
        },
        store_path,
    );
    super::run_log_lines::outcome_lines(&log, &out, started.elapsed());
    out
}

fn run_at_inner(request: RunRequest<'_>, store_path: Option<&Path>) -> Outcome {
    let RunRequest {
        a,
        root_a,
        b,
        root_b,
        opts,
        filter,
        cancel,
        settings,
        observer,
    } = request;
    let endpoints = SyncEndpoints::new(a, root_a, b, root_b);
    let finish = |mut out: Outcome| {
        out.canceled = cancel.load(Ordering::Acquire);
        out
    };
    if let Err(error) = crate::vfs::validate_sync_roots(a, root_a, b, root_b) {
        return finish(failure("Sync-Pfade", error));
    }
    let id = pair_lock_id(a, root_a, b, root_b);
    let lock = match super::PairLock::acquire_wait(&id, settings.lock_wait, cancel) {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            return finish(Outcome {
                busy: true,
                ..Outcome::default()
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            return finish(Outcome::default())
        }
        Err(error) => return finish(failure("Paarsperre", error)),
    };
    let _identity_locks = match super::backend_identity_migration::migrate(&lock, endpoints, cancel)
    {
        Ok(locks) => locks,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            return finish(Outcome {
                busy: true,
                ..Outcome::default()
            });
        }
        Err(error) => return finish(failure("Backend-Identität", error)),
    };
    let replicas = match super::replica::identify(endpoints, &settings, !opts.dry_run) {
        Ok(replicas) => replicas,
        Err(error) => return finish(failure("Laufwerk-Erkennung", error)),
    };
    let key = &replicas.key;
    if let Some(block) = replicas.blocked {
        return finish(Outcome {
            blocked: Some(block),
            state: Some(key.clone()),
            ..Outcome::default()
        });
    }
    if !opts.dry_run {
        if let Err(error) = super::replacement_recovery::recover_locked(
            &lock,
            key,
            endpoints,
            opts.cross_mounts,
            cancel,
        ) {
            return finish(Outcome {
                state: Some(key.clone()),
                ..failure("Replacement-Wiederanlauf", error)
            });
        }
    }
    let path = match super::baseline_file(key) {
        Ok(path) => path,
        Err(error) => return finish(failure("Synchronisierungsstand", error)),
    };
    // Import only jobs proven to predate RV1, once per owner. A new job
    // never inherits an old pair's deletions merely because endpoints match.
    let import = match &key.owner {
        super::StateOwner::AdHoc => true,
        super::StateOwner::Job(id) => match crate::syncjobs::legacy_baseline_pending(id) {
            Ok(pending) => pending,
            Err(error) => return finish(failure("Altzustand-Migration", error)),
        },
    };
    let state_exists = path.exists() || path.with_extension("journal").exists();
    let mut transient_base = None;
    if !state_exists {
        if let Some(previous) = replicas.upgraded_from.as_ref() {
            let keys = pair_key_policy(a, root_a, b, root_b);
            match super::checkpoint_journal::Journal::load(previous, keys) {
                Ok((_, records, dirs)) => {
                    if opts.dry_run {
                        transient_base = Some(records.baseline);
                    } else {
                        let saved = super::save_baseline(&path, &records.baseline).and_then(|()| {
                            if let Some(dirs) = dirs {
                                super::state_metadata::save_dirs(key, &dirs)
                            } else {
                                Ok(())
                            }
                        });
                        if let Err(error) = saved {
                            return finish(failure("Replika-Markierung", error));
                        }
                    }
                }
                Err(error) => return finish(failure("Replika-Markierung", error)),
            }
        }
    }
    if import
        && !state_exists
        && replicas.upgraded_from.is_none()
        && replicas.history.is_none()
        && !key.is_legacy()
    {
        match super::load_baseline(&super::baseline_path(&key.pair_id)) {
            Ok(base) if opts.dry_run => transient_base = Some(base),
            Ok(base) => {
                if let Err(error) = super::save_baseline(&path, &base) {
                    return finish(failure("Altzustand-Migration", error));
                }
            }
            Err(error) => return finish(failure("Altzustand-Migration", error)),
        }
    }
    let (mut journal, records, dirs) = match super::checkpoint_journal::Journal::load(
        key,
        pair_key_policy(a, root_a, b, root_b),
    ) {
        Ok(loaded) => loaded,
        Err(error) => return finish(failure("Synchronisierungsstand", error)),
    };
    if !opts.dry_run && state_exists {
        if let Err(error) = journal.compact(
            key,
            &records,
            dirs.as_ref().unwrap_or(&super::DirSet::new()),
        ) {
            return finish(failure("Zwischenstand-Wiederaufnahme", error));
        }
    }
    let baseline = transient_base.unwrap_or(records.baseline);
    let versions = super::versions::RunVersions::begin(super::versions::VersionsContext::new(
        &key.pair_id,
        key.owner.clone(),
        opts.versions,
        opts.versioning,
    ));
    let state = RunState {
        endpoints,
        opts,
        settings: &settings,
        cancel,
        filter,
        observer,
        lock: &lock,
        key,
        history: replicas.history.as_ref(),
        baseline: &baseline,
        dirs: dirs.as_ref(),
        versions: &versions,
        store_path,
    };
    let due = opts.verify_target_secs > 0
        && state.history.is_none_or(|history| {
            let elapsed = super::state_metadata::now_ms()
                .saturating_sub(history.full_ms)
                .max(0) as u64;
            history.full_ms == 0 || elapsed >= opts.verify_target_secs.saturating_mul(1000)
        });
    let mut out = if settings.depth != super::ScanDepth::Full && !replicas.changed && !due {
        super::incremental::try_incremental_run(&state)
            .unwrap_or_else(|| super::orchestration_full::run_full_locked(&state))
    } else {
        super::orchestration_full::run_full_locked(&state)
    };
    out.state = Some(key.clone());
    out.run_id = Some(versions.run_id().to_string());
    if !opts.dry_run && out.blocked.is_none() {
        for result in [
            versions.finish(),
            super::versions::prune_after_run(
                &lock,
                &key.pair_id,
                &[
                    super::versions::VersionSide {
                        side: super::PairSide::A,
                        backend: a,
                        root: root_a,
                    },
                    super::versions::VersionSide {
                        side: super::PairSide::B,
                        backend: b,
                        root: root_b,
                    },
                ],
                &opts.versioning,
                &AtomicBool::new(false),
            ),
        ] {
            if let Err(error) = result {
                out.errors.push(("Versionen".into(), error.to_string()));
                out.stats.errors = out.stats.errors.saturating_add(1);
            }
        }
    }
    finish(out)
}

pub(super) fn failure(stage: &str, error: impl std::fmt::Display) -> Outcome {
    Outcome {
        errors: vec![(stage.to_string(), error.to_string())],
        stats: super::BisyncStats {
            errors: 1,
            ..super::BisyncStats::default()
        },
        ..Outcome::default()
    }
}
