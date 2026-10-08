//! One lightweight receiver per real-time job. Filesystem/network discovery
//! never runs on the scheduling or heartbeat thread. Wakeups remain durable.
use crate::bisync::PairSide;
use crate::syncjobs::{
    ChangeDetection, PendingKind, PendingTrigger, RunCause, SyncJob, Trigger, WatchStatus,
};
use crate::watch::{Coverage, WatchEvent, WatchHandle, WatchId, WatchMessage};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

struct Worker {
    fingerprint: String,
    stop: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

pub(super) struct Realtime {
    workers: HashMap<String, Worker>,
    sender: crossbeam_channel::Sender<(String, RunCause, i64)>,
    receiver: crossbeam_channel::Receiver<(String, RunCause, i64)>,
    ready: HashMap<String, (RunCause, i64)>,
}
impl Realtime {
    pub(super) fn new() -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(4096);
        Self {
            workers: HashMap::new(),
            sender,
            receiver,
            ready: HashMap::new(),
        }
    }
    pub(super) fn refresh(&mut self, jobs: &[SyncJob], enabled: bool) {
        let states = crate::syncjobs::load_job_states(jobs);
        self.workers.retain(|id, _| {
            enabled
                && jobs.iter().any(|job| {
                    &job.id == id
                        && job.enabled
                        && job.trigger == Trigger::RealTime
                        && states.get(id).is_some_and(|state| {
                            state.load_error.is_none()
                                && !state
                                    .last_error
                                    .as_ref()
                                    .is_some_and(|error| error.kind.needs_user())
                        })
                })
        });
        for job in jobs
            .iter()
            .filter(|job| enabled && job.enabled && job.trigger == Trigger::RealTime)
        {
            if !states.get(&job.id).is_some_and(|state| {
                state.load_error.is_none()
                    && !state
                        .last_error
                        .as_ref()
                        .is_some_and(|error| error.kind.needs_user())
            }) {
                continue;
            }
            let fingerprint = format!("{job:?}");
            if self
                .workers
                .get(&job.id)
                .is_some_and(|worker| worker.fingerprint == fingerprint)
            {
                continue;
            }
            self.workers.remove(&job.id);
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = stop.clone();
            let sender = self.sender.clone();
            let configured = job.clone();
            match std::thread::Builder::new()
                .name("sync-watch-job".into())
                .spawn(move || receive(configured, worker_stop, sender))
            {
                Ok(_) => {
                    self.workers
                        .insert(job.id.clone(), Worker { fingerprint, stop });
                }
                Err(error) => {
                    super::state::log(&format!("watch '{}' cannot start: {error}", job.name))
                }
            }
        }
    }
    pub(super) fn ready(&mut self, jobs: &[SyncJob]) -> HashMap<String, RunCause> {
        for (id, cause, since) in self.receiver.try_iter() {
            self.ready.insert(id, (cause, since));
        }
        self.ready.retain(|id, _| {
            jobs.iter().any(|job| &job.id == id && job.enabled)
                && crate::syncjobs::load_job_state(id)
                    .ok()
                    .is_some_and(|state| state.pending_trigger.is_some())
        });
        self.ready
            .iter()
            .filter_map(|(id, ready)| {
                let state = crate::syncjobs::load_job_state(id).ok()?;
                settled(state.pending_trigger.as_ref()?, *ready).then(|| (id.clone(), ready.0))
            })
            .collect()
    }
}

fn settled(pending: &PendingTrigger, (cause, since): (RunCause, i64)) -> bool {
    pending.since <= since
        && match pending.kind {
            PendingKind::Change => matches!(cause, RunCause::Change | RunCause::Poll),
            PendingKind::Verify => true,
            _ => false,
        }
}

fn signal(
    id: &str,
    cause: RunCause,
    sink: &crossbeam_channel::Sender<(String, RunCause, i64)>,
) -> bool {
    let Ok(state) = crate::syncjobs::load_job_state(id) else {
        return false;
    };
    let Some(pending) = state.pending_trigger else {
        return true;
    };
    sink.try_send((id.to_string(), cause, pending.since))
        .is_ok()
}

fn receive(
    job: SyncJob,
    stop: Arc<AtomicBool>,
    sink: crossbeam_channel::Sender<(String, RunCause, i64)>,
) {
    let (local_tx, local_rx) = crossbeam_channel::bounded::<WatchMessage>(4096);
    let (remote_tx, remote_rx) = crossbeam_channel::bounded(4096);
    let overflow = Arc::new(AtomicBool::new(false));
    let filter = match super::job_triggers::filter(&job, super::platform::watch_case_fold()) {
        Ok(filter) => filter,
        Err(error) => {
            super::state::log(&error);
            return;
        }
    };
    let mut local: HashMap<WatchId, (PairSide, WatchHandle)> = HashMap::new();
    // None = no events; Some(false) = partial; Some(true) = complete.
    let mut coverage: HashMap<PairSide, Option<bool>> = HashMap::new();
    for (side, endpoint) in super::job_triggers::sides(&job) {
        coverage.insert(side, None);
        if let Some(root) = super::schedule::local_root(&endpoint) {
            match crate::watch::watch(
                &root,
                crate::watch::WatchOptions {
                    cross_mounts: job.cross_mounts,
                },
                filter.clone(),
                local_tx.clone().into(),
            ) {
                Ok(handle) => {
                    local.insert(handle.id(), (side, handle));
                }
                Err(error) => super::state::log(&format!("watch '{}': {error}", job.name)),
            }
        } else {
            super::remote_watch::start(
                endpoint,
                side,
                job.rt_poll_secs,
                stop.clone(),
                overflow.clone(),
                remote_tx.clone(),
            );
        }
    }
    let mut dirty: Option<(Instant, Instant)> = None;
    let mut changed = Vec::new();
    let mut uncertain = false;
    let mut verification = true; // control scan after every new subscription
    let mut last_poll = Instant::now();
    let mut note = None;
    let mut previous_status = None;
    let mut persisted_dirty = None;
    let debounce = Duration::from_secs(job.rt_debounce_secs);
    let max_wait = Duration::from_secs(job.effective_rt_max_latency_secs());
    while !stop.load(Ordering::Acquire) {
        for message in local_rx.try_iter() {
            let Some((side, _)) = local.get(&message.id) else {
                continue;
            };
            match message.event {
                WatchEvent::Ready(kind) => {
                    coverage.insert(*side, Some(kind == Coverage::Complete));
                    verification = true;
                }
                WatchEvent::Unavailable(reason) => {
                    coverage.insert(*side, None);
                    note = Some(format!("{reason:?}"));
                }
                WatchEvent::Overflow => {
                    verification = true;
                    uncertain = true;
                }
                WatchEvent::Change(change) => {
                    uncertain |= change.rel.is_empty();
                    changed.push((*side, change.rel));
                    mark_dirty(&job.id, &mut dirty);
                }
            }
        }
        for message in remote_rx.try_iter() {
            use crate::vfs::ChangeNotice;
            match message.event {
                ChangeNotice::Ready { .. } => {
                    // `Ready` promises complete coverage of the root (pushed
                    // notices or a complete polled feed such as Drive's);
                    // partial subscriptions send `ReadyPartial`.
                    coverage.insert(message.side, Some(true));
                    verification = true;
                }
                ChangeNotice::ReadyPartial { .. } => {
                    coverage.insert(message.side, Some(false));
                    verification = true;
                }
                ChangeNotice::Changed { paths, .. } => {
                    if paths.is_empty() {
                        uncertain = true;
                        mark_dirty(&job.id, &mut dirty);
                    }
                    for path in paths {
                        if path.is_empty()
                            || filter.admits(&crate::watch::WatchEntry {
                                rel: &path,
                                is_dir: None,
                            })
                        {
                            uncertain |= path.is_empty();
                            changed.push((message.side, path));
                            mark_dirty(&job.id, &mut dirty);
                        }
                    }
                }
                ChangeNotice::Overflow => {
                    verification = true;
                    uncertain = true;
                }
                ChangeNotice::Ended(error) => {
                    coverage.insert(message.side, None);
                    note = Some(error);
                    uncertain = true;
                    mark_dirty(&job.id, &mut dirty);
                }
            }
        }
        if overflow.swap(false, Ordering::AcqRel) {
            verification = true;
            uncertain = true;
        }
        // Bound event coalescing as well as the channel; overflow always verifies.
        if changed.len() > 4096 {
            changed.clear();
            uncertain = true;
            verification = true;
        }
        let complete = coverage.values().all(|value| *value == Some(true));
        let events = coverage.values().any(Option::is_some);
        let detection = if complete {
            ChangeDetection::Events
        } else if events {
            ChangeDetection::EventsAndPoll {
                poll_secs: job.rt_poll_secs,
            }
        } else {
            ChangeDetection::Poll {
                poll_secs: job.rt_poll_secs,
            }
        };
        let status = (
            detection.clone(),
            if complete { None } else { note.clone() },
        );
        if previous_status.as_ref() != Some(&status) {
            let _ = crate::syncjobs::update_job_state(&job.id, |state| {
                state.watch = Some(WatchStatus {
                    detection,
                    since: super::state::now_secs(),
                    note: status.1.clone(),
                });
            });
            previous_status = Some(status);
        }
        if !complete
            && job.rt_poll_secs > 0
            && last_poll.elapsed() >= Duration::from_secs(job.rt_poll_secs)
        {
            last_poll = Instant::now();
            uncertain = true;
            mark_dirty(&job.id, &mut dirty);
        }
        if dirty.is_some_and(|(_, last)| Some(last) != persisted_dirty) {
            if super::job_triggers::persist_change(&job.id, super::state::now_secs()) {
                persisted_dirty = dirty.map(|(_, last)| last);
            }
        }
        if verification {
            verification = !(super::job_triggers::persist(
                &job.id,
                PendingKind::Verify,
                super::state::now_secs(),
                None,
            ) && signal(&job.id, RunCause::Verify, &sink));
        }
        if dirty.is_some_and(|(first, last)| {
            Some(last) == persisted_dirty
                && (last.elapsed() >= debounce || first.elapsed() >= max_wait)
        }) {
            let written_state = crate::syncjobs::load_job_state(&job.id).ok();
            let generation = written_state.as_ref().and_then(|state| state.last_attempt);
            let pending_since = written_state
                .as_ref()
                .and_then(|state| state.pending_trigger.as_ref().map(|pending| pending.since));
            if !uncertain
                && !changed.is_empty()
                && written_state
                    .as_ref()
                    .is_some_and(|state| state.running.is_some())
                && changed.iter().all(|(side, rel)| {
                    super::own_writes::candidate(
                        &job.id,
                        *side,
                        rel,
                        match side {
                            PairSide::A => &job.source,
                            PairSide::B => &job.target,
                        },
                        generation,
                    )
                })
            {
                // These may be our writes, but a cancelled attempt must still
                // retain its original unfinished work. Decide after it ends.
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            let own_only = !uncertain
                && !changed.is_empty()
                && changed.iter().all(|(side, rel)| {
                    super::own_writes::matches(
                        &job.id,
                        *side,
                        rel,
                        match side {
                            PairSide::A => &job.source,
                            PairSide::B => &job.target,
                        },
                        generation,
                        &stop,
                    )
                });
            if !own_only {
                if !signal(&job.id, RunCause::Change, &sink) {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
            } else {
                // Never clear a change arriving during a later run or a
                // different trigger. The existing run already wrote these.
                let mut cleared = false;
                let stored = crate::syncjobs::update_job_state(&job.id, |state| {
                    if state.running.is_none() && state.last_attempt == generation {
                        if let Some(pending) = state.pending_trigger.as_ref() {
                            if pending.kind != PendingKind::Change {
                                cleared = true;
                            } else if Some(pending.since) == pending_since {
                                state.pending_trigger = None;
                                cleared = true;
                            }
                        } else {
                            cleared = true;
                        }
                    }
                });
                if stored.is_err() || !cleared {
                    persisted_dirty = None;
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
            }
            changed.clear();
            uncertain = false;
            dirty = None;
            persisted_dirty = None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_task_new_events_wait_for_their_own_debounce_but_cancellation_keeps_settled_work() {
        let mut pending = PendingTrigger {
            kind: PendingKind::Change,
            since: 100,
            volume: None,
        };
        let ready = (RunCause::Change, 100);
        assert!(settled(&pending, ready)); // an aborted run leaves this unchanged
        pending.since = 101; // a change after the previous run started
        assert!(!settled(&pending, ready));
        assert!(settled(&pending, (RunCause::Change, 101)));
        assert!(!settled(&pending, (RunCause::Verify, 101)));
    }
}
fn mark_dirty(id: &str, dirty: &mut Option<(Instant, Instant)>) {
    let now = Instant::now();
    match dirty {
        Some((_, last)) => *last = now,
        None => *dirty = Some((now, now)),
    }
    let _ = id;
}
