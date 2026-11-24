//! The input of the controlling viewer for the app's accessibility reader
//! (`screenControl(0, …)`, `ScreenHostControl.ensureInput`).
//!
//! Review 2026-11-19 (round 3): `send_input` runs in the session's input
//! path (`host_session_control.rs`, `inject_off_runtime`), which the session
//! loop awaits before it produces the next picture. It never waits: a backlog
//! of pointer moves or wheel steps is merged, everything else is queued up to
//! `HARD_LIMIT`, and only beyond that a batch is refused, which the session
//! reports as a block (never a silent loss of a key or button).
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::json;

use crate::screen::core::input_model::{InputEvent, TouchPhase};

/// From this many waiting batches on, a batch of pointer moves (or of wheel
/// steps) is merged into a waiting one of the same kind at the end of the
/// queue: the app performs one stroke at a time, so a backlog means it falls
/// behind. Below it every move keeps its place (drag paths stay exact).
const MERGE_FROM: usize = 8;
/// Batches the queue holds at most; a batch beyond is refused.
pub(super) const HARD_LIMIT: usize = 128;

/// A batch and the host session (bridge channel) it belongs to; 0: a batch
/// of the backend without a session (`BridgeBackend::inject`).
struct Batch {
    /// Its number (from 1, `NEXT_BATCH`): the app names it when it reports
    /// the batch performed or refused (`screen.hostNotice {inputBatch}`,
    /// input receipts, round 12 rest).
    id: u64,
    session: u64,
    events: Vec<InputEvent>,
    /// The batch lets go of something (a key or button up, a touch that ends):
    /// it is never dropped while a reader runs, so the app's gesture state
    /// (`GestureMapper`: a stroke ends with its release) never keeps a press
    /// of a session that lost control or ended.
    release: bool,
    /// `false`: a release of a session that lost control or ended, kept only
    /// so that the app ends that session's stroke in its gesture state; the
    /// app performs nothing of it (no tap, text or action for another
    /// session's viewer, review 2026-11-19 round 4, Y1).
    perform: bool,
}

/// Keeps `batch` of a session that lost control or ended only when it
/// releases something, and then only for clean-up (`perform: false`).
fn keep_for_cleanup(batch: &mut Batch) -> bool {
    batch.perform = false;
    batch.release
}

/// Whether `events` let go of a key, a button or a touch.
fn releases(events: &[InputEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            InputEvent::Key { down: false, .. }
                | InputEvent::Button { down: false, .. }
                | InputEvent::Touch {
                    phase: TouchPhase::End | TouchPhase::Cancel,
                    ..
                }
        )
    })
}

static INPUT: Mutex<VecDeque<Batch>> = Mutex::new(VecDeque::new());
/// The number of the next queued batch.
static NEXT_BATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_batch() -> u64 {
    NEXT_BATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
static READY: Condvar = Condvar::new();

fn queue() -> MutexGuard<'static, VecDeque<Batch>> {
    INPUT.lock().unwrap_or_else(PoisonError::into_inner)
}

fn pointer_only(batch: &[InputEvent]) -> bool {
    !batch.is_empty()
        && batch
            .iter()
            .all(|event| matches!(event, InputEvent::PointerAbs { .. }))
}

fn wheel_only(batch: &[InputEvent]) -> bool {
    !batch.is_empty()
        && batch
            .iter()
            .all(|event| matches!(event, InputEvent::Wheel { .. }))
}

fn wheel_sum(batch: &[InputEvent]) -> (i16, i16) {
    batch
        .iter()
        .fold((0i16, 0i16), |(x, y), event| match event {
            InputEvent::Wheel { dx, dy } => (x.saturating_add(*dx), y.saturating_add(*dy)),
            _ => (x, y),
        })
}

/// Merges `batch` into the waiting `tail` when both are pointer moves only
/// (the newest position wins) or wheel steps only at the same position (the
/// steps add up).
fn merge(tail: &mut Vec<InputEvent>, batch: &[InputEvent]) -> bool {
    if pointer_only(tail) && pointer_only(batch) {
        *tail = batch[batch.len() - 1..].to_vec();
        return true;
    }
    if wheel_only(tail) && wheel_only(batch) {
        let (ax, ay) = wheel_sum(tail);
        let (bx, by) = wheel_sum(batch);
        *tail = vec![InputEvent::Wheel {
            dx: ax.saturating_add(bx),
            dy: ay.saturating_add(by),
        }];
        return true;
    }
    false
}

/// Queues the input of the controlling viewer of host session `session`
/// without waiting: the number of the batch that carries it (a merged batch
/// keeps the number of the one it joined); `None` when the queue holds
/// `HARD_LIMIT` batches and `events` merges with none.
pub fn send_input(session: u64, events: &[InputEvent]) -> Option<u64> {
    push(queue(), session, events)
}

/// [`send_input`] for a batch of the session that holds control now (its
/// input path, not its release job). Control is one session's at a time
/// (`registry.rs`, `take_control`), and the session checks it before every
/// batch (`host_session_control.rs`, `holds_control`): any other session's
/// batch still waiting belongs to a viewer that gave control up meanwhile,
/// so it is dropped instead of performed for the new holder (review
/// 2026-11-19, Codex 3/3); a batch that releases something stays for
/// clean-up only (`perform: false`).
pub fn send_controlling_input(session: u64, events: &[InputEvent]) -> Option<u64> {
    let mut queue = queue();
    queue.retain_mut(|batch| {
        batch.session == session || batch.session == 0 || keep_for_cleanup(batch)
    });
    push(queue, session, events)
}

/// The last release of `session` when it gives control up or ends
/// (`ScreenInputSink::revoke`, `host_input_release.rs`, `drain_releases`;
/// round 7: not the releases of a session that keeps steering): its batches still
/// waiting and the release itself only end the strokes in the app's gesture
/// state (`perform: false`). Nothing the session queued before is performed
/// after it let go, and the release of a held button performs no stroke
/// either (review 2026-11-19, Codex 6/4).
pub fn send_release(session: u64, events: &[InputEvent]) -> bool {
    let mut queue = queue();
    for batch in queue.iter_mut().filter(|batch| batch.session == session) {
        batch.perform = false;
    }
    if events.is_empty() {
        return true;
    }
    if queue.len() >= HARD_LIMIT {
        return false;
    }
    queue.push_back(Batch {
        id: next_batch(),
        session,
        events: events.to_vec(),
        release: true,
        perform: false,
    });
    READY.notify_all();
    true
}

fn push(
    mut queue: MutexGuard<'static, VecDeque<Batch>>,
    session: u64,
    events: &[InputEvent],
) -> Option<u64> {
    if queue.len() >= MERGE_FROM {
        if let Some(tail) = queue
            .back_mut()
            .filter(|tail| tail.session == session && tail.perform)
        {
            if merge(&mut tail.events, events) {
                READY.notify_all();
                return Some(tail.id);
            }
        }
    }
    if queue.len() >= HARD_LIMIT {
        return None;
    }
    let id = next_batch();
    queue.push_back(Batch {
        id,
        session,
        events: events.to_vec(),
        release: releases(events),
        perform: true,
    });
    READY.notify_all();
    Some(id)
}

/// The next batch as the app reads it (`{"op":"input","batch":…,
/// "session":…,"perform":…,"events":[…]}`; the app performs it only while
/// that session exists and `perform` holds, feeds every batch to that
/// session's own gesture state and reports a performed batch's number),
/// waiting at most `timeout`.
pub(super) fn take(timeout: Duration) -> Option<String> {
    let queue = queue();
    let (mut queue, _) = READY
        .wait_timeout_while(queue, timeout, |queue| queue.is_empty())
        .unwrap_or_else(PoisonError::into_inner);
    queue.pop_front().map(|batch| {
        json!({
            "op": "input",
            "batch": batch.id,
            "session": batch.session,
            "perform": batch.perform,
            "events": batch.events,
        })
        .to_string()
    })
}

/// Drops the waiting batches of host session `session` (it ended): its input
/// is never performed for another session (review 2026-11-19, Codex 3/3).
/// Its releases stay for clean-up only (`perform: false`): the app ends the
/// session's stroke in its gesture state and performs nothing of it
/// (`ScreenHostControl.ensureInput`).
pub(super) fn drop_session(session: u64) {
    queue().retain_mut(|batch| batch.session != session || keep_for_cleanup(batch));
}

/// Drops every waiting batch, releases too (the last session ended: the
/// app's reader stops and its gesture state goes with it).
pub(super) fn clear() {
    queue().clear();
}
