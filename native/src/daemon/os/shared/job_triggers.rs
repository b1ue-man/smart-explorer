//! Shared trigger policy: endpoint directions, filters and durable wakeups.
use crate::bisync::{Direction, PairSide};
use crate::syncjobs::{FailureKind, JobState, PendingKind, PendingTrigger, SyncJob};
use crate::watch::{WatchEntry, WatchFilter};

pub(super) fn sides(job: &SyncJob) -> Vec<(PairSide, String)> {
    match job.direction {
        Direction::AtoB => vec![(PairSide::A, job.source.clone())],
        Direction::BtoA => vec![(PairSide::B, job.target.clone())],
        Direction::Both => vec![
            (PairSide::A, job.source.clone()),
            (PairSide::B, job.target.clone()),
        ],
    }
}

pub(super) fn filter(job: &SyncJob, fold_case: bool) -> Result<WatchFilter, String> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in &job.ignore {
        builder.add(
            globset::GlobBuilder::new(pattern)
                .case_insensitive(fold_case)
                .build()
                .map_err(|error| error.to_string())?,
        );
    }
    let ignore = builder.build().map_err(|error| error.to_string())?;
    let hidden = job.include_hidden;
    Ok(WatchFilter::new(move |entry: &WatchEntry<'_>| {
        if entry.rel.split('/').any(|name| {
            crate::bisync::is_engine_name(name)
                || crate::vfs::is_staging_name(name)
                || (!hidden && name.starts_with('.'))
        }) {
            return false;
        }
        !ignore.is_match(entry.rel) && !ignore.is_match(format!("{}/", entry.rel))
    }))
}

pub(super) fn persist(id: &str, kind: PendingKind, now: i64, volume: Option<String>) -> bool {
    match crate::syncjobs::update_job_state(id, |state| {
        merge(state, kind, now, volume);
    }) {
        Ok(_) => true,
        Err(error) => {
            super::state::log(&format!("trigger for '{id}' cannot be stored: {error}"));
            false
        }
    }
}

fn merge(state: &mut JobState, kind: PendingKind, now: i64, volume: Option<String>) {
    // Timer evaluation repeats while an admitted job is queued/running.
    // It is still the same occurrence, not a fresh change during the run.
    if kind == PendingKind::Other
        && state
            .pending_trigger
            .as_ref()
            .is_some_and(|pending| pending.kind == PendingKind::Other)
    {
        return;
    }
    // Even a stale/cancelling run may still own blocking I/O. Its
    // later completion must not consume events that arrived after start.
    let running = state.running.as_ref();
    let after_start = running.map_or(now, |mark| mark.started.saturating_add(1).max(now));
    let since = match &state.pending_trigger {
        Some(previous) if running.is_some_and(|mark| previous.since <= mark.started) => after_start,
        Some(previous) => previous.since.min(after_start),
        None => after_start,
    };
    // Keep the stronger kind while moving its generation beyond a run
    // already using it. Keeping only the old timestamp would lose changes.
    if let Some(previous) = state.pending_trigger.as_mut().filter(|pending| {
        pending.kind == PendingKind::Confirmed
            || (pending.kind == PendingKind::Verify
                && matches!(kind, PendingKind::Change | PendingKind::Other))
    }) {
        previous.since = since;
        return;
    }
    state.pending_trigger = Some(PendingTrigger {
        kind,
        since,
        volume,
    });
}

pub(super) fn persist_change(id: &str, now: i64) -> bool {
    persist(id, PendingKind::Change, now, None)
}

pub(super) fn host_cursor(job: &SyncJob) -> Option<String> {
    let mut cursors = Vec::new();
    for (side, endpoint) in sides(job) {
        let Some(root) = super::schedule::local_root(&endpoint) else {
            return None;
        };
        let cursor = crate::watch::host_cursor(&root)?;
        cursors.push((side.as_str(), endpoint, cursor));
    }
    serde_json::to_string(&cursors).ok()
}

/// Resolver still exposes text errors. Prefer conservative authentication
/// classification to avoid locking accounts; V-REMOTE owns typed resolution.
pub(super) fn connect_failure(message: &str) -> FailureKind {
    crate::syncjobs::classify_failure(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_one_way_watches_only_the_source_and_applies_job_filters() {
        let mut job = SyncJob::new("x".into(), "sftp://a/x".into(), "sftp://b/x".into());
        job.direction = Direction::AtoB;
        assert_eq!(sides(&job), vec![(PairSide::A, "sftp://a/x".into())]);
        job.include_hidden = true;
        job.ignore = vec!["Skip/**".into()];
        let filter = filter(&job, true).unwrap();
        for name in ["skip/a", ".se-versions/run/a", ".se-sync-replica"] {
            assert!(!filter.admits(&WatchEntry {
                rel: name,
                is_dir: None
            }));
        }
        assert!(filter.admits(&WatchEntry {
            rel: "node_modules/a",
            is_dir: None
        }));
        assert_eq!(connect_failure("Authentication failed"), FailureKind::Auth);
    }
    #[test]
    fn review_task_pending_generation_moves_beyond_a_running_attempt() {
        let mut state = JobState {
            running: Some(crate::syncjobs::RunMark {
                runner: crate::syncjobs::Runner::Daemon,
                cause: crate::syncjobs::RunCause::Verify,
                started: 100,
                alive: 100,
                stalled_since: None,
            }),
            pending_trigger: Some(PendingTrigger {
                kind: PendingKind::Verify,
                since: 90,
                volume: None,
            }),
            ..JobState::default()
        };
        merge(&mut state, PendingKind::Change, 100, None);
        assert_eq!(
            state
                .pending_trigger
                .as_ref()
                .map(|pending| (pending.kind, pending.since)),
            Some((PendingKind::Verify, 101))
        );
        merge(&mut state, PendingKind::Change, 110, None);
        assert_eq!(
            state.pending_trigger.as_ref().map(|pending| pending.since),
            Some(101)
        );
        // Stale marks must be respected too: their I/O may still complete.
        merge(&mut state, PendingKind::Change, 1000, None);
        assert_eq!(
            state.pending_trigger.as_ref().map(|pending| pending.since),
            Some(101)
        );
        state.pending_trigger = None;
        merge(&mut state, PendingKind::Other, 100, None);
        merge(&mut state, PendingKind::Other, 110, None);
        assert_eq!(
            state.pending_trigger.as_ref().map(|pending| pending.since),
            Some(101)
        );
    }
}
