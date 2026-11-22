//! Releases a session could not deliver before it gave up the control or
//! ended (review 2026-11-19, Codex 7/1): they leave the session for this
//! keeper, which delivers them in order (the running job first, then what
//! failed and what waited), and while it holds any, no session injects
//! input (`busy`, `Session::hold_reason`): a new holder's first press never
//! meets a late release of the one before. The keeper gives up after
//! `GATE_LIMIT` (named in the host log) so that a dead sink cannot keep
//! everyone's input waiting.
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

/// How long the keeper tries, from `FRAME_TIMEOUT` + 2 × `INPUT_TIMEOUT`
/// (10 s + 2 × 2 s, `host_worker_client.rs`): a host process's release
/// waits for the link a picture request holds, then for its answer; a
/// second try after a refusal gets its own answer time.
const GATE_LIMIT: Duration = Duration::from_secs(10 + 2 * 2);
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

/// Takes over the running `jobs` (an injection that outlasted its grace,
/// a release job; their failures land in `failed`), in their order, and
/// the releases still `queued`.
pub(super) fn take_over(
    jobs: Vec<(tokio::task::JoinHandle<Result<(), ControlBlock>>, Duration)>,
    failed: Arc<Mutex<Vec<InputEvent>>>,
    mut queued: Vec<InputEvent>,
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
        // Round 11 (M2): each job gets its own limit (a text takes its
        // characters' time); a job past it is awaited again before the
        // keeper ends, as it may still put failed releases into `failed`.
        let mut late = Vec::new();
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
            if events.is_empty() {
                if late.is_empty() {
                    return;
                }
                for job in late.drain(..) {
                    let _ = tokio::time::timeout(GATE_LIMIT, job).await;
                }
                started = Instant::now();
                continue;
            }
            let target = target.clone();
            let delivery = tokio::task::spawn_blocking(move || deliver(&target, events));
            let Ok(joined) = tokio::time::timeout(GATE_LIMIT, delivery).await else {
                super::super::visibility::note_viewer(
                    "releases of a former holder did not finish; given up",
                );
                return;
            };
            let left = joined.unwrap_or_default();
            if left.is_empty() {
                continue;
            }
            failed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(left);
            // Round 8 (I4): a sink whose host process ended takes nothing
            // more; nobody's input waits for it.
            let closed = matches!(&target, Target::Sink(sink) if sink.closed());
            if closed || started.elapsed() >= GATE_LIMIT {
                super::super::visibility::note_viewer("releases of a former holder were given up");
                return;
            }
            tokio::time::sleep(RETRY).await;
        }
    });
}

/// Delivers `events` in chunks a host process takes; what did not go.
fn deliver(target: &Target, events: Vec<InputEvent>) -> Vec<InputEvent> {
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
    events[done..].to_vec()
}
