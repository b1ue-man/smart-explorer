//! Concurrent reporting: no new admission after a checkpoint or target stop.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use crate::transfer::engine::folders::FolderRegister;
use crate::transfer::Side;
use super::apply::ApplyReport;
use super::apply_actions::Actions;
use super::apply_retry::{run_with_retry, AttemptError};
use super::checkpoint::ApplyScope;
use super::completion::DirAction;
use super::incremental::SyncEndpoints;
use super::run_types::RunStop;
use super::sync_flows::PairFlows;
use super::types::{Action, BisyncOptions, BisyncStats, PairSide, Throttle, Tree};

pub(super) fn merge(left: &mut BisyncStats, right: &BisyncStats) {
    left.a_to_b = left.a_to_b.saturating_add(right.a_to_b);
    left.b_to_a = left.b_to_a.saturating_add(right.b_to_a);
    left.deleted = left.deleted.saturating_add(right.deleted);
    left.bytes = left.bytes.saturating_add(right.bytes);
    left.errors = left.errors.saturating_add(right.errors);
}

pub(super) fn target_side(action: &Action) -> PairSide {
    match action {
        Action::CopyAtoB(_) | Action::KeepBothAtoB(_) | Action::DeleteB(_) => PairSide::B,
        Action::CopyBtoA(_) | Action::KeepBothBtoA(_) | Action::DeleteA(_) => PairSide::A,
        Action::FinalizeMoveAtoB(_) => PairSide::A,
        Action::FinalizeMoveBtoA(_) => PairSide::B,
    }
}
pub(super) fn classify(error: &io::Error, rel: &str, side: PairSide,
    scope: &ApplyScope<'_>, stopped: &AtomicBool) -> bool {
    if super::apply_guard::is_drift(error) {
        scope.sink.deferred(rel, &error.to_string());
        return true;
    }
    if let Some(kind) = super::apply_boundary::omitted(error) {
        scope.sink.omitted(rel, kind);
        return true;
    }
    let stop = match error.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => Some(RunStop::TargetFull { side }),
        io::ErrorKind::ReadOnlyFilesystem => Some(RunStop::TargetReadOnly { side }),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected | io::ErrorKind::BrokenPipe | io::ErrorKind::TimedOut
            | io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable | io::ErrorKind::NetworkDown
            => Some(RunStop::ConnectionLost { side }),
        _ => None,
    };
    if let Some(stop) = stop { stopped.store(true, Ordering::Release); scope.sink.stopped(stop); }
    false
}
fn dry(actions: &[Action]) -> ApplyReport {
    let mut stats = BisyncStats::default();
    for action in actions {
        match action {
            Action::CopyAtoB(_) | Action::KeepBothAtoB(_) => stats.a_to_b += 1,
            Action::CopyBtoA(_) | Action::KeepBothBtoA(_) => stats.b_to_a += 1,
            _ => stats.deleted += 1,
        }
    }
    ApplyReport { stats, completed: Vec::new() }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(actions: &[Action], dirs: &[DirAction], planned: Option<(&Tree, &Tree)>,
    endpoints: SyncEndpoints<'_>, opts: BisyncOptions, scope: &ApplyScope<'_>,
    errors: &mut Vec<(String, String)>, cancel: &AtomicBool, allow_deferred: bool,
) -> ApplyReport {
    if opts.dry_run { return dry(actions); }
    let mut report = ApplyReport::default();
    if let Err(error) = scope.versions.bind_lock(&super::pair_lock_id(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b)) {
        report.stats.errors = 1; errors.push(("Versionen".into(), error.to_string())); return report;
    }
    let stopped = AtomicBool::new(false);
    let directory_pass = |creating: bool, report: &mut ApplyReport, errors: &mut Vec<(String,String)>| {
        for action in dirs.iter().filter(|action| matches!(action, DirAction::Create {..}) == creating) {
            if cancel.load(Ordering::Acquire) || scope.sink.should_stop() || stopped.load(Ordering::Acquire) { break; }
            if let Err(error) = super::apply_dirs::run(action, endpoints, opts, scope, cancel) {
                if !classify(&error, action.rel(), action.side(), scope, &stopped) {
                    report.stats.errors += 1;
                    if errors.len() < 100 { errors.push((action.rel().to_string(), error.to_string())); }
                }
            }
        }
    };
    directory_pass(true, &mut report, errors);
    let flows = PairFlows::new(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b);
    let folders_a = FolderRegister::new(Side::Remote(endpoints.a), endpoints.root_a, flows.flow(PairSide::A).clone());
    let folders_b = FolderRegister::new(Side::Remote(endpoints.b), endpoints.root_b, flows.flow(PairSide::B).clone());
    let throttle = Throttle::new(opts.bwlimit_bps);
    // One preflight proves that a later real root syncfs can cover a Deferred
    // stage. With nested mounts or move semantics, use immediate confirmation.
    let mut deferred = [false; 2];
    if allow_deferred && !opts.cross_mounts && !opts.move_files {
        for (index, backend, root) in [(0, endpoints.a, endpoints.root_a), (1, endpoints.b, endpoints.root_b)] {
            if backend.is_local() {
                match crate::vfs::sync_filesystem(backend, root) {
                    Ok(available) => deferred[index] = available,
                    Err(error) => {
                        classify(&error, root, if index == 0 { PairSide::A } else { PairSide::B }, scope, &stopped);
                        errors.push((root.to_string(), error.to_string()));
                        report.stats.errors += 1;
                        stopped.store(true, Ordering::Release);
                    }
                }
            }
        }
    }
    let stop = || stopped.load(Ordering::Acquire) || scope.sink.should_stop();
    let runner = Actions { endpoints, opts, scope, planned, throttle: &throttle, cancel, deferred };
    let completed = Mutex::new(Vec::new());
    let partial = Mutex::new(BisyncStats::default());
    let admit = |action: &Action| {
        if cancel.load(Ordering::Acquire) || stop() { return None; }
        let rel = super::core::action_rel(action);
        let side = target_side(action);
        let copy = matches!(action, Action::CopyAtoB(_) | Action::CopyBtoA(_) | Action::KeepBothAtoB(_) | Action::KeepBothBtoA(_));
        if copy {
            let (backend, root, folders, source, source_root) = if side == PairSide::A {
                (endpoints.a, endpoints.root_a, &folders_a, endpoints.b, endpoints.root_b)
            } else { (endpoints.b, endpoints.root_b, &folders_b, endpoints.a, endpoints.root_a) };
            let spelling = scope.spellings.side_rel(rel, side);
            let source_rel = scope.spellings.side_rel(rel, side.other());
            if super::apply_boundary::guard(source, source_root, source_rel, opts.cross_mounts).is_ok()
                && super::apply_boundary::guard(backend, root, spelling, opts.cross_mounts).is_ok()
                && super::apply_boundary::target(backend, root, spelling, None).is_ok() {
                if let Some((parent,_)) = spelling.rsplit_once('/') {
                    if let Ok(path) = crate::vfs::sync_path(backend, root, parent) {
                        let prefix = format!("{}/", root.trim_end_matches('/'));
                        if let Some(encoded) = path.strip_prefix(&prefix) {
                            if !stop() { let _ = folders.ensure(encoded, cancel); }
                        }
                    }
                }
            }
        }
        if stop() { return None; }
        match action { Action::DeleteA(_) => flows.single(PairSide::A, cancel),
            Action::DeleteB(_) => flows.single(PairSide::B, cancel), _ => flows.transfer(cancel) }
    };
    let execute = |action: &Action| {
        let mut stats = BisyncStats::default();
        let result = run_with_retry(opts.retries, Duration::from_secs(opts.retry_delay_secs), cancel,
            || runner.run(action, &mut stats));
        match result {
            Ok(()) => {
                completed.lock().unwrap_or_else(|e| e.into_inner()).push(action.clone());
                Ok(stats)
            }
            Err(error) => {
                merge(&mut partial.lock().unwrap_or_else(|e| e.into_inner()), &stats);
                if classify(error.error(), super::core::action_rel(action), target_side(action), scope, &stopped) {
                    Ok(BisyncStats::default())
                } else { Err(error) }
            }
        }
    };
    let cap = match (opts.max_transfers, flows.shared_connection_cap(endpoints.a, endpoints.b)) {
        (0, shared) => shared.unwrap_or(0),
        (max, shared) => shared.map_or(max, |shared| shared.min(max)),
    };
    let pooled = super::apply_pool::run_actions_stoppable(actions, cap, cancel, &stop, &admit, &execute);
    merge(&mut report.stats, &pooled.stats);
    merge(&mut report.stats, &partial.into_inner().unwrap_or_else(|e| e.into_inner()));
    errors.extend(pooled.errors);
    report.completed = completed.into_inner().unwrap_or_else(|e| e.into_inner());
    directory_pass(false, &mut report, errors);
    report
}
