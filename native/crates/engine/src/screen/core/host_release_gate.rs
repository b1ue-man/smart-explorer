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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Hand-overs still delivering.
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

/// How long the keeper tries: a host process's release ends within 12 s or
/// its process is ended (`FRAME_TIMEOUT` + `INPUT_TIMEOUT`), so two jobs
/// and the retries between them fit [assumption: 30 s; not measured].
const GATE_LIMIT: Duration = Duration::from_secs(30);
/// The pause between two tries of releases the sink refused.
const RETRY: Duration = Duration::from_secs(1);

/// Releases of a former holder are still on their way: input waits.
pub(in super::super) fn busy() -> bool {
    OUTSTANDING.load(Ordering::Acquire) > 0
}

/// Where the releases go: the source's sink, or the backend without one.
#[derive(Clone)]
pub(super) enum Target {
    Sink(Arc<dyn ScreenInputSink>),
    Backend(Arc<dyn ScreenHostBackend>),
}

struct Outstanding;
impl Drop for Outstanding {
    fn drop(&mut self) {
        OUTSTANDING.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Takes over the running release `job` (its failures land in `failed`)
/// and the releases still `queued`.
pub(super) fn take_over(
    job: Option<tokio::task::JoinHandle<Result<(), ControlBlock>>>,
    failed: Arc<Mutex<Vec<InputEvent>>>,
    mut queued: Vec<InputEvent>,
    target: Target,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        eprintln!("screen session: releases of a former holder could not be kept");
        return;
    };
    OUTSTANDING.fetch_add(1, Ordering::AcqRel);
    let outstanding = Outstanding;
    runtime.spawn(async move {
        let _outstanding = outstanding;
        let started = Instant::now();
        if let Some(job) = job {
            let _ = job.await;
        }
        loop {
            let mut events =
                std::mem::take(&mut *failed.lock().unwrap_or_else(PoisonError::into_inner));
            events.append(&mut queued);
            if events.is_empty() {
                return;
            }
            let target = target.clone();
            let left = tokio::task::spawn_blocking(move || deliver(&target, events))
                .await
                .unwrap_or_default();
            if left.is_empty() {
                continue;
            }
            failed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(left);
            if started.elapsed() >= GATE_LIMIT {
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
        let result = match target {
            Target::Sink(sink) => sink.release(chunk),
            Target::Backend(backend) => backend.inject(chunk),
        };
        if result.is_err() {
            break;
        }
        done += chunk.len();
    }
    events[done..].to_vec()
}
