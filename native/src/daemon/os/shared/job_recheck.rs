//! Keeps job lines current between runs: evidence that a failure only the
//! user could fix may be gone, and run marks whose runner ended without a
//! result. Neither starts a login on its own: a `needs_user` failure is
//! retried only after a stored credential changed or another job on the same
//! OAuth account succeeded later (no lockout risk: that success proves the
//! shared token works).
use std::collections::{BTreeMap, HashSet};

use crate::syncjobs::{
    FailureKind, Interrupted, JobState, PendingKind, PendingTrigger, Recheck, SyncJob,
};

/// Updates the stored states; `true` when any state changed.
pub(super) fn refresh(
    jobs: &[SyncJob],
    states: &BTreeMap<String, JobState>,
    now: i64,
    active: &HashSet<String>,
) -> bool {
    let credentials = crate::creds::credentials_revision();
    let mut changed = false;
    for job in jobs {
        let Some(state) = states
            .get(&job.id)
            .filter(|state| state.load_error.is_none())
        else {
            continue;
        };
        if let Some((evidence, reason)) = evidence(job, state, jobs, states, credentials) {
            changed |= store_recheck(job, evidence, reason);
        }
        if !active.contains(&job.id) {
            changed |= clear_dead_mark(job, state, now);
        }
    }
    changed
}

fn uses_drive(job: &SyncJob) -> bool {
    [&job.source, &job.target].iter().any(|endpoint| {
        endpoint
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("gdrive://")
    })
}

/// The newest evidence after the failed attempt that the job's own retry
/// has not used yet.
fn evidence(
    job: &SyncJob,
    state: &JobState,
    jobs: &[SyncJob],
    states: &BTreeMap<String, JobState>,
    credentials: Option<i64>,
) -> Option<(i64, String)> {
    let error = state.last_error.as_ref()?;
    if state.consecutive_failures == 0 || error.kind != FailureKind::Auth {
        return None;
    }
    let attempt = state.last_attempt?;
    let used = state
        .recheck
        .as_ref()
        .map_or(i64::MIN, |recheck| recheck.evidence);
    let mut best = credentials
        .filter(|at| *at > attempt)
        .map(|at| (at, "Anmeldedaten wurden geändert".to_string()));
    if uses_drive(job) {
        let sibling = jobs
            .iter()
            .filter(|other| other.id != job.id && uses_drive(other))
            .filter_map(|other| {
                let success = states.get(&other.id)?.last_success?;
                (success > attempt).then(|| (success, other))
            })
            .max_by_key(|(success, _)| *success);
        if let Some((success, other)) = sibling {
            if best.as_ref().is_none_or(|(at, _)| success > *at) {
                let name = if other.name.trim().is_empty() {
                    other.id.as_str()
                } else {
                    other.name.trim()
                };
                best = Some((
                    success,
                    format!("Sync „{name}“ mit demselben Google-Konto war danach erfolgreich"),
                ));
            }
        }
    }
    best.filter(|(at, _)| *at > used)
}

fn store_recheck(job: &SyncJob, evidence: i64, reason: String) -> bool {
    let mut stored = false;
    let result = crate::syncjobs::update_job_state(&job.id, |state| {
        if state
            .last_error
            .as_ref()
            .is_some_and(|error| error.kind == FailureKind::Auth)
            && state
                .recheck
                .as_ref()
                .is_none_or(|recheck| recheck.evidence < evidence)
        {
            state.recheck = Some(Recheck {
                evidence,
                reason: reason.clone(),
                pending: true,
            });
            stored = true;
        }
    });
    match result {
        Ok(_) if stored => {
            crate::bisync::job_log_line(
                &job.id,
                "Wiederholung",
                &format!("{reason}; ein neuer Versuch ist vorgemerkt"),
            );
            true
        }
        Ok(_) => false,
        Err(error) => {
            super::state::log(&format!(
                "recheck for '{}' cannot be stored: {error}",
                job.id
            ));
            false
        }
    }
}

/// A run mark nobody renews any more becomes `interrupted`; an automatic job
/// gets a verification run for whatever that run left undone.
fn clear_dead_mark(job: &SyncJob, state: &JobState, now: i64) -> bool {
    let Some(mark) = state.running.clone() else {
        return false;
    };
    if state.running_now(now).is_some() {
        return false;
    }
    let mut cleared = false;
    let result = crate::syncjobs::update_job_state(&job.id, |state| {
        if state.running.as_ref() != Some(&mark) {
            return;
        }
        state.running = None;
        state.interrupted = Some(Interrupted {
            runner: mark.runner,
            started: mark.started,
            alive: mark.alive,
            detected: now,
        });
        if state.pending_trigger.is_none() {
            state.pending_trigger = Some(PendingTrigger {
                kind: PendingKind::Verify,
                since: now,
                volume: None,
            });
        }
        cleared = true;
    });
    match result {
        Ok(_) if cleared => {
            crate::bisync::job_log_line(
                &job.id,
                "Unterbrochen",
                &format!(
                    "Lauf ({:?}, gestartet {}) hat sich seit {} nicht mehr gemeldet und gilt als unterbrochen; ein Kontrolllauf ist vorgemerkt",
                    mark.runner,
                    local(mark.started),
                    local(mark.alive)
                ),
            );
            true
        }
        Ok(_) => false,
        Err(error) => {
            super::state::log(&format!(
                "run mark of '{}' cannot be cleared: {error}",
                job.id
            ));
            false
        }
    }
}

fn local(secs: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| secs.to_string())
}

#[cfg(test)]
#[path = "sync_transparency_task_recheck_tests.rs"]
mod sync_transparency_task_recheck_tests;
