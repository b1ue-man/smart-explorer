//! Concurrent reporting: no new admission after a checkpoint or target stop.
//! Plan rules of a job kind may run the actions in phases (`run_phases`).
use super::apply::ApplyReport;
use super::apply_actions::Actions;
use super::apply_pool::PoolReport;
use super::apply_retry::{run_with_retry, AttemptError};
use super::checkpoint::ApplyScope;
use super::completion::{ApplySink, DirAction};
use super::incremental::SyncEndpoints;
use super::plan_rules::{PhasePlan, PlanRules, HELD_BACK};
use super::run_types::RunStop;
use super::sync_flows::PairFlows;
use super::types::{Action, BisyncOptions, BisyncStats, PairSide, Throttle, Tree};
use crate::transfer::engine::folders::FolderRegister;
use crate::transfer::Side;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

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
pub(super) fn classify(
    error: &io::Error,
    rel: &str,
    side: PairSide,
    scope: &ApplyScope<'_>,
    stopped: &AtomicBool,
) -> bool {
    if super::apply_guard::is_drift(error) {
        scope.sink.deferred(rel, &error.to_string());
        return true;
    }
    if let Some(kind) = super::apply_boundary::omitted(error) {
        scope.sink.omitted(rel, kind);
        // The read classifier also describes failed writes and backups. Keep
        // their old basis protected, but do not report the action as successful.
        return !matches!(
            kind,
            super::omissions::OmissionKind::Unreadable
                | super::omissions::OmissionKind::Vanished
                | super::omissions::OmissionKind::NotRepresentable
        );
    }
    let stop = match error.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
            Some(RunStop::TargetFull { side })
        }
        io::ErrorKind::ReadOnlyFilesystem => Some(RunStop::TargetReadOnly { side }),
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::NotConnected
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::TimedOut
        | io::ErrorKind::HostUnreachable
        | io::ErrorKind::NetworkUnreachable
        | io::ErrorKind::NetworkDown => Some(RunStop::ConnectionLost { side }),
        _ => None,
    };
    if let Some(stop) = stop {
        stopped.store(true, Ordering::Release);
        scope.sink.stopped(stop);
    }
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
    ApplyReport {
        stats,
        completed: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    actions: &[Action],
    dirs: &[DirAction],
    planned: Option<(&Tree, &Tree)>,
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    scope: &ApplyScope<'_>,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
    allow_deferred: bool,
) -> ApplyReport {
    if opts.dry_run {
        return dry(actions);
    }
    let mut report = ApplyReport::default();
    if let Err(error) = scope.versions.bind_lock(&super::pair_lock_id(
        endpoints.a,
        endpoints.root_a,
        endpoints.b,
        endpoints.root_b,
    )) {
        report.stats.errors = 1;
        errors.push(("Versionen".into(), error.to_string()));
        return report;
    }
    let stopped = AtomicBool::new(false);
    let directory_pass =
        |creating: bool, report: &mut ApplyReport, errors: &mut Vec<(String, String)>| {
            for action in dirs
                .iter()
                .filter(|action| matches!(action, DirAction::Create { .. }) == creating)
            {
                if cancel.load(Ordering::Acquire)
                    || scope.sink.should_stop()
                    || stopped.load(Ordering::Acquire)
                {
                    break;
                }
                if let Err(error) = super::apply_dirs::run(action, endpoints, opts, scope, cancel) {
                    if !classify(&error, action.rel(), action.side(), scope, &stopped) {
                        report.stats.errors += 1;
                        if errors.len() < 100 {
                            errors.push((action.rel().to_string(), error.to_string()));
                        }
                    }
                }
            }
        };
    directory_pass(true, &mut report, errors);
    let flows = PairFlows::new(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b);
    let folders_a = FolderRegister::new(
        Side::Remote(endpoints.a),
        endpoints.root_a,
        flows.flow(PairSide::A).clone(),
    );
    let folders_b = FolderRegister::new(
        Side::Remote(endpoints.b),
        endpoints.root_b,
        flows.flow(PairSide::B).clone(),
    );
    let throttle = Throttle::new(opts.bwlimit_bps);
    // One preflight proves that a later real root syncfs can cover a Deferred
    // stage. With nested mounts or move semantics, use immediate confirmation.
    let mut deferred = [false; 2];
    if allow_deferred && !opts.cross_mounts && !opts.move_files {
        for (index, backend, root) in [
            (0, endpoints.a, endpoints.root_a),
            (1, endpoints.b, endpoints.root_b),
        ] {
            if backend.is_local() {
                match crate::vfs::sync_filesystem(backend, root) {
                    Ok(available) => deferred[index] = available,
                    Err(error) => {
                        classify(
                            &error,
                            root,
                            if index == 0 { PairSide::A } else { PairSide::B },
                            scope,
                            &stopped,
                        );
                        errors.push((root.to_string(), error.to_string()));
                        report.stats.errors += 1;
                        stopped.store(true, Ordering::Release);
                    }
                }
            }
        }
    }
    let stop = || stopped.load(Ordering::Acquire) || scope.sink.should_stop();
    let runner = Actions {
        endpoints,
        opts,
        scope,
        planned,
        throttle: &throttle,
        cancel,
        deferred,
    };
    let completed = Mutex::new(Vec::new());
    let partial = Mutex::new(BisyncStats::default());
    let admit = |action: &Action| {
        if cancel.load(Ordering::Acquire) || stop() {
            return None;
        }
        let rel = super::core::action_rel(action);
        let side = target_side(action);
        let copy = matches!(
            action,
            Action::CopyAtoB(_)
                | Action::CopyBtoA(_)
                | Action::KeepBothAtoB(_)
                | Action::KeepBothBtoA(_)
        );
        if copy {
            let (backend, root, folders, source, source_root) = if side == PairSide::A {
                (
                    endpoints.a,
                    endpoints.root_a,
                    &folders_a,
                    endpoints.b,
                    endpoints.root_b,
                )
            } else {
                (
                    endpoints.b,
                    endpoints.root_b,
                    &folders_b,
                    endpoints.a,
                    endpoints.root_a,
                )
            };
            let spelling = scope.spellings.side_rel(rel, side);
            let source_rel = scope.spellings.side_rel(rel, side.other());
            if super::apply_boundary::guard(source, source_root, source_rel, opts.cross_mounts)
                .is_ok()
                && super::apply_boundary::guard(backend, root, spelling, opts.cross_mounts).is_ok()
                && super::apply_boundary::target(backend, root, spelling, None).is_ok()
            {
                if let Some((parent, _)) = spelling.rsplit_once('/') {
                    if let Ok(path) = crate::vfs::sync_path(backend, root, parent) {
                        let prefix = format!("{}/", root.trim_end_matches('/'));
                        if let Some(encoded) = path.strip_prefix(&prefix) {
                            if !stop() {
                                let _ = folders.ensure(encoded, cancel);
                            }
                        }
                    }
                }
            }
        }
        if stop() {
            return None;
        }
        match action {
            Action::DeleteA(_) => flows.single(PairSide::A, cancel),
            Action::DeleteB(_) => flows.single(PairSide::B, cancel),
            _ => flows.transfer(cancel),
        }
    };
    let execute = |action: &Action| {
        let mut stats = BisyncStats::default();
        let result = run_with_retry(
            opts.retries,
            Duration::from_secs(opts.retry_delay_secs),
            cancel,
            || runner.run(action, &mut stats),
        );
        match result {
            Ok(()) => {
                completed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(action.clone());
                Ok(stats)
            }
            Err(error) => {
                merge(
                    &mut partial.lock().unwrap_or_else(|e| e.into_inner()),
                    &stats,
                );
                if classify(
                    error.error(),
                    super::core::action_rel(action),
                    target_side(action),
                    scope,
                    &stopped,
                ) && error.before_commit()
                {
                    Ok(BisyncStats::default())
                } else {
                    Err(error)
                }
            }
        }
    };
    let cap = match (
        opts.max_transfers,
        flows.shared_connection_cap(endpoints.a, endpoints.b),
    ) {
        (0, shared) => shared.unwrap_or(0),
        (max, shared) => shared.map_or(max, |shared| shared.min(max)),
    };
    // Installed plan rules may split apply into phases (C5.4 R4); without
    // rules, or with their single phase in plan order, the one pool call
    // below is exactly the call every job kind made before.
    let phased = super::plan_rules::current()
        .and_then(|rules| super::plan_rules::phase_plan(&rules, actions).map(|plan| (rules, plan)));
    let pooled = match &phased {
        None => {
            super::apply_pool::run_actions_stoppable(actions, cap, cancel, &stop, &admit, &execute)
        }
        Some((rules, plan)) => run_phases(
            actions,
            plan,
            rules,
            scope.sink,
            &completed,
            &|| cancel.load(Ordering::Acquire) || stop(),
            &mut |phase| {
                super::apply_pool::run_actions_stoppable(
                    phase, cap, cancel, &stop, &admit, &execute,
                )
            },
        ),
    };
    merge(&mut report.stats, &pooled.stats);
    merge(
        &mut report.stats,
        &partial.into_inner().unwrap_or_else(|e| e.into_inner()),
    );
    errors.extend(pooled.errors);
    report.completed = completed.into_inner().unwrap_or_else(|e| e.into_inner());
    directory_pass(false, &mut report, errors);
    report
}

/// The first failures kept across all phases (the bound of one pool run).
const PHASE_ERRORS: usize = 100;

/// Identity of an action for matching finished actions to planned ones.
fn action_key(action: &Action) -> (u8, &str) {
    let kind = match action {
        Action::CopyAtoB(_) => 0,
        Action::CopyBtoA(_) => 1,
        Action::FinalizeMoveAtoB(_) => 2,
        Action::FinalizeMoveBtoA(_) => 3,
        Action::DeleteA(_) => 4,
        Action::DeleteB(_) => 5,
        Action::KeepBothAtoB(_) => 6,
        Action::KeepBothBtoA(_) => 7,
    };
    (kind, super::core::action_rel(action))
}

/// Applies the phases one after another (C5.4 R4): a phase starts only after
/// every action of the previous one completed or failed. Actions in no
/// phase, and actions the rules hold back after an incomplete earlier
/// action, are not started and are reported as deferred: nothing is
/// committed for them and the next run plans them again. A stop or cancel
/// ends the phases like it ends admission within one phase.
fn run_phases(
    actions: &[Action],
    plan: &PhasePlan,
    rules: &dyn PlanRules,
    sink: &dyn ApplySink,
    completed: &Mutex<Vec<Action>>,
    halted: &dyn Fn() -> bool,
    apply: &mut dyn FnMut(&[Action]) -> PoolReport,
) -> PoolReport {
    let mut report = PoolReport::default();
    let mut incomplete: Vec<&Action> = Vec::new();
    for &index in &plan.deferred {
        let action = &actions[index];
        sink.deferred(
            super::core::action_rel(action),
            &rules.deferral_reason(action),
        );
        incomplete.push(action);
    }
    for phase in &plan.phases {
        if halted() {
            break;
        }
        let coming: Vec<&Action> = phase.iter().map(|&index| &actions[index]).collect();
        let mut held = vec![false; coming.len()];
        if !incomplete.is_empty() {
            for position in rules.held_back(&incomplete, &coming) {
                if let Some(flag) = held.get_mut(position) {
                    *flag = true;
                }
            }
        }
        let mut indices = Vec::with_capacity(phase.len());
        for (&index, &hold) in phase.iter().zip(&held) {
            if hold {
                let action = &actions[index];
                sink.deferred(super::core::action_rel(action), HELD_BACK);
                incomplete.push(action);
            } else {
                indices.push(index);
            }
        }
        if indices.is_empty() {
            continue;
        }
        let run: Vec<Action> = indices
            .iter()
            .map(|&index| actions[index].clone())
            .collect();
        let before = completed.lock().unwrap_or_else(|e| e.into_inner()).len();
        let pooled = apply(&run[..]);
        merge(&mut report.stats, &pooled.stats);
        let room = PHASE_ERRORS.saturating_sub(report.errors.len());
        report.errors.extend(pooled.errors.into_iter().take(room));
        report.completed.extend(pooled.completed);
        let finished: Vec<Action> =
            completed.lock().unwrap_or_else(|e| e.into_inner())[before..].to_vec();
        let mut done: HashMap<(u8, &str), usize> = HashMap::new();
        for action in &finished {
            *done.entry(action_key(action)).or_default() += 1;
        }
        for &index in &indices {
            match done.get_mut(&action_key(&actions[index])) {
                Some(count) if *count > 0 => *count -= 1,
                _ => incomplete.push(&actions[index]),
            }
        }
    }
    report
}

#[cfg(test)]
#[path = "apply_phases_tests.rs"]
mod phases_tests;
