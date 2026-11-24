//! Releases a session could not deliver before it gave up the control or
//! ended (review 2026-11-19, Codex 7/1): they leave the session for this
//! keeper, which delivers them in order (the running job first, then what
//! failed and what waited), and while it holds any, no session injects
//! input (`busy`, `Session::hold_reason`): a new holder's first press never
//! meets a late release of the one before. The keeper gives up after
//! `GATE_LIMIT` (named in the host log) so that a dead sink cannot keep
//! everyone's input waiting; a job past its limit is awaited first (at most
//! `GATE_LIMIT`), and what it failed gets one more `GATE_LIMIT` of tries
//! (round 12, S3: the keeper ended with it still running, and its failed
//! releases stayed unread).
use crate::screen::core::host_backend::{ScreenHostBackend, ScreenInputSink};
use crate::screen::core::input_model::{InputEvent, MAX_INPUT_EVENTS};
use crate::screen::core::wire_control::ControlBlock;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Hand-overs still delivering, per input target (round 8, I4).
static OUTSTANDING: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

fn outstanding() -> std::sync::MutexGuard<'static, Option<HashMap<String, usize>>> {
    OUTSTANDING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How long the keeper tries (`worker_limits::GATE_LIMIT`): a host
/// process's release waits for the link a picture request holds, then for
/// its answer; a second try after a refusal gets its own answer time.
use crate::screen::core::worker_limits::GATE_LIMIT;
/// The pause between two tries of releases the sink refused.
const RETRY: Duration = Duration::from_secs(1);

/// Releases of a former holder of `target` are still on their way: input
/// into it waits; other targets go on.
pub(in super::super) fn busy(target: &str) -> bool {
    outstanding()
        .as_ref()
        .is_some_and(|map| map.get(target).is_some_and(|count| *count > 0))
}

/// Where the releases go: the source's sink, or the backend without one.
#[derive(Clone)]
pub(super) enum Target {
    Sink(Arc<dyn ScreenInputSink>),
    Backend(Arc<dyn ScreenHostBackend>),
}

struct Outstanding(String);
impl Drop for Outstanding {
    fn drop(&mut self) {
        if let Some(map) = outstanding().as_mut() {
            if let Some(count) = map.get_mut(&self.0) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    map.remove(&self.0);
                }
            }
        }
    }
}

type Job = tokio::task::JoinHandle<Result<(), ControlBlock>>;

/// Takes over the running `jobs` (an injection that outlasted its grace,
/// a release job; their failures land in `failed`), in their order, and
/// the releases still `queued`; `owes`: the sink may owe releases of its
/// own (round 12, Codex 3), it is nudged until it confirms.
pub(super) fn take_over(
    jobs: Vec<(Job, Duration)>,
    (failed, mut queued, mut owes): (Arc<Mutex<Vec<InputEvent>>>, Vec<InputEvent>, bool),
    target: Target,
    key: String,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        eprintln!("screen session: releases of a former holder could not be kept");
        return;
    };
    *outstanding()
        .get_or_insert_with(HashMap::new)
        .entry(key.clone())
        .or_insert(0) += 1;
    let outstanding = Outstanding(key);
    runtime.spawn(async move {
        let _outstanding = outstanding;
        // Round 10: a job that never ends does not keep everyone's input
        // waiting. Its releases still follow: they go through the same sink,
        // which serialises them behind it (the host process's link lock,
        // Android's queue, the portal thread), so they cannot overtake it.
        // Round 11 (M2): each job gets its own limit (round 12, S1: an
        // injection one call's time, it stops before its next call); a job
        // past it is awaited again before the keeper ends, as it may still
        // put failed releases into `failed`.
        let mut late: Vec<Job> = Vec::new();
        for (mut job, limit) in jobs {
            if tokio::time::timeout(limit, &mut job).await.is_err() {
                super::super::visibility::note_viewer(
                    "a handed-over input job outlasted its limit; its releases follow it",
                );
                late.push(job);
            }
        }
        // Round 9 (E4): the limit counts from here, after the handed jobs.
        let mut started = Instant::now();
        loop {
            let mut events =
                std::mem::take(&mut *failed.lock().unwrap_or_else(PoisonError::into_inner));
            events.append(&mut queued);
            if events.is_empty() && !owes {
                if late.is_empty() {
                    return;
                }
                await_late(&mut late).await;
                started = Instant::now();
                continue;
            }
            let sink = target.clone();
            let delivery = tokio::task::spawn_blocking(move || deliver(&sink, events));
            let Ok(joined) = tokio::time::timeout(GATE_LIMIT, delivery).await else {
                // Round 12 (S3): a late job's failures still follow.
                if late.is_empty() {
                    super::super::visibility::note_viewer(
                        "releases of a former holder did not finish; given up",
                    );
                    return;
                }
                await_late(&mut late).await;
                started = Instant::now();
                continue;
            };
            let (left, went) = joined.unwrap_or_default();
            owes = !went && matches!(target, Target::Sink(_));
            if went {
                continue;
            }
            failed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(left);
            // Round 8 (I4): a sink whose host process ended takes nothing
            // more; nobody's input waits for it.
            let closed = matches!(&target, Target::Sink(sink) if sink.closed());
            if closed || (started.elapsed() >= GATE_LIMIT && late.is_empty()) {
                super::super::visibility::note_viewer("releases of a former holder were given up");
                return;
            }
            if started.elapsed() >= GATE_LIMIT {
                // Round 12 (S3): a late job may still fail releases into
                // `failed`; they get their own tries after it.
                await_late(&mut late).await;
                started = Instant::now();
                continue;
            }
            tokio::time::sleep(RETRY).await;
        }
    });
}

/// Waits for the jobs past their limit (each at most `GATE_LIMIT`).
async fn await_late(late: &mut Vec<Job>) {
    for job in late.drain(..) {
        let _ = tokio::time::timeout(GATE_LIMIT, job).await;
    }
}

/// Delivers `events` in chunks a host process takes: what did not go, and
/// whether all went. No events: a nudge for the sink's own debts (Codex 3),
/// which went when the sink answers without a refusal.
fn deliver(target: &Target, events: Vec<InputEvent>) -> (Vec<InputEvent>, bool) {
    if events.is_empty() {
        let went = match target {
            Target::Sink(sink) => sink.revoke(&[]).is_ok(),
            Target::Backend(_) => true,
        };
        return (events, went);
    }
    let mut done = 0;
    for chunk in events.chunks(MAX_INPUT_EVENTS) {
        // Round 8 (I6): the giving-up session's own releases: a queueing
        // sink (Android) also takes back its input still waiting.
        let result = match target {
            Target::Sink(sink) => sink.revoke(chunk),
            Target::Backend(backend) => backend.inject(chunk),
        };
        if result.is_err() {
            break;
        }
        done += chunk.len();
    }
    let left = events[done..].to_vec();
    let went = left.is_empty();
    (left, went)
}
