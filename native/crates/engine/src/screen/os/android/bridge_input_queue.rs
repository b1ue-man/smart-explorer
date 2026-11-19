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

use crate::screen::core::input_model::InputEvent;

/// From this many waiting batches on, a batch of pointer moves (or of wheel
/// steps) is merged into a waiting one of the same kind at the end of the
/// queue: the app performs one stroke at a time, so a backlog means it falls
/// behind. Below it every move keeps its place (drag paths stay exact).
const MERGE_FROM: usize = 8;
/// Batches the queue holds at most; a batch beyond is refused.
const HARD_LIMIT: usize = 128;

static INPUT: Mutex<VecDeque<Vec<InputEvent>>> = Mutex::new(VecDeque::new());
static READY: Condvar = Condvar::new();

fn queue() -> MutexGuard<'static, VecDeque<Vec<InputEvent>>> {
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

/// Queues the input of the controlling viewer without waiting; `false` when
/// the queue holds `HARD_LIMIT` batches and `events` merges with none.
pub fn send_input(events: &[InputEvent]) -> bool {
    let mut queue = queue();
    if queue.len() >= MERGE_FROM {
        if let Some(tail) = queue.back_mut() {
            if merge(tail, events) {
                READY.notify_all();
                return true;
            }
        }
    }
    if queue.len() >= HARD_LIMIT {
        return false;
    }
    queue.push_back(events.to_vec());
    READY.notify_all();
    true
}

/// The next batch as the app reads it (`{"op":"input","events":[…]}`),
/// waiting at most `timeout`.
pub(super) fn take(timeout: Duration) -> Option<String> {
    let queue = queue();
    let (mut queue, _) = READY
        .wait_timeout_while(queue, timeout, |queue| queue.is_empty())
        .unwrap_or_else(PoisonError::into_inner);
    queue
        .pop_front()
        .map(|events| json!({"op": "input", "events": events}).to_string())
}

/// Drops every waiting batch (the last session ended).
pub(super) fn clear() {
    queue().clear();
}
