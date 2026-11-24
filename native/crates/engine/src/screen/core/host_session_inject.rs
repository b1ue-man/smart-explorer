//! The end of a batch's injection (review 2026-11-19, Codex 7/10): the
//! session loop takes it from `settle_input` on every viewer frame and
//! capture tick, applies it and starts the batches that waited, in order.
use super::*;
use crate::screen::core::input_receipt::InputResult;

/// Events a waiting batch may absorb (round 8, I2; round 9, E6): pointer
/// positions, wheel steps and touch moves.
fn mergeable(event: &InputEvent) -> bool {
    matches!(
        event,
        InputEvent::PointerAbs { .. }
            | InputEvent::Wheel { .. }
            | InputEvent::Touch {
                phase: crate::screen::core::input_model::TouchPhase::Move,
                ..
            }
    )
}

use super::super::input_release::releases;

/// `a` and `b` concern the same key, button or touch.
fn same_input(a: &InputEvent, b: &InputEvent) -> bool {
    match (a, b) {
        (InputEvent::Key { hid_usage: x, .. }, InputEvent::Key { hid_usage: y, .. }) => x == y,
        (InputEvent::Button { button: x, .. }, InputEvent::Button { button: y, .. }) => x == y,
        (InputEvent::Touch { id: x, .. }, InputEvent::Touch { id: y, .. }) => x == y,
        _ => false,
    }
}

pub(super) fn motion_only(events: &[InputEvent]) -> bool {
    events.iter().all(mergeable)
}

/// A merged batch holds at most this many events; beyond, the next batch
/// waits on its own.
const MERGED_MAX: usize = 16;
/// Events waiting behind an injection at most (round 9, E6); beyond, only
/// the releases of what is held or waits wait (one per key, button or
/// touch), the rest is dropped and the viewer hears `BacklogFull`.
pub(super) const BACKLOG_EVENTS: usize = 1024;

/// `events` joins `waiting` (both `motion_only`) in order: a position
/// replaces the position just before it, wheel steps add to the steps just
/// before them, a touch move replaces the same touch's move just before it
/// (round 9, E3: a wheel step stays at the position it was made at).
/// `false` (and `waiting` unchanged) when the result would exceed
/// `MERGED_MAX`.
pub(super) fn merge_motion(waiting: &mut Vec<InputEvent>, events: &[InputEvent]) -> bool {
    let mut merged = waiting.clone();
    for event in events {
        match (merged.last_mut(), event) {
            (Some(last @ InputEvent::PointerAbs { .. }), InputEvent::PointerAbs { .. }) => {
                *last = event.clone()
            }
            (Some(InputEvent::Wheel { dx, dy }), InputEvent::Wheel { dx: x, dy: y }) => {
                *dx = dx.saturating_add(*x);
                *dy = dy.saturating_add(*y);
            }
            (Some(last @ InputEvent::Touch { .. }), InputEvent::Touch { id, .. }) if matches!(&*last, InputEvent::Touch { id: held, .. } if held == id) => {
                *last = event.clone()
            }
            _ => merged.push(event.clone()),
        }
    }
    if merged.len() > MERGED_MAX {
        return false;
    }
    *waiting = merged;
    true
}

impl Session<'_> {
    /// Round 11 (M1, N7): of a batch beyond the backlog's bound, the
    /// releases of what is held (`pressed`) or pressed in the injection or
    /// the backlog, each once: a release whose press was dropped would let
    /// go of a key the host's own user holds, and a repeated one adds
    /// nothing but length.
    pub(super) fn overflow_releases(&self, events: &[InputEvent]) -> Vec<InputEvent> {
        let mut kept: Vec<InputEvent> = Vec::new();
        for event in events.iter().filter(|event| releases(event)) {
            let same = |other: &&InputEvent| same_input(other, event);
            let waiting = self
                .health
                .injecting
                .iter()
                .flat_map(|job| job.events.iter());
            let last = waiting
                .chain(self.health.backlog.iter().flatten())
                .chain(kept.iter())
                .filter(same)
                .last();
            let keep = match last {
                Some(last) => !releases(last),
                None => self.pressed.clone().take_release(event),
            };
            if keep {
                kept.push(event.clone());
            }
        }
        kept
    }

    /// A press waits behind the injection or is in it (round 10: moves are
    /// then not merged, a drag keeps its path).
    pub(super) fn presses_pending(&self) -> bool {
        let press = |event: &InputEvent| {
            matches!(
                event,
                InputEvent::Key { down: true, .. }
                    | InputEvent::Button { down: true, .. }
                    | InputEvent::Touch {
                        phase: crate::screen::core::input_model::TouchPhase::Start,
                        ..
                    }
            )
        };
        self.health.backlog.iter().flatten().any(press)
            || self
                .health
                .injecting
                .as_ref()
                .is_some_and(|job| job.events.iter().any(press))
    }

    /// Takes the result of a finished injection and starts the batches that
    /// waited, in their order (on every viewer frame and capture tick).
    pub(in super::super) async fn settle_input(&mut self) -> io::Result<()> {
        // Round 12 rest: batches a queueing sink's app finished.
        self.poll_receipts().await?;
        loop {
            if let Some(job) = self.health.injecting.as_mut() {
                use std::future::Future;
                let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
                let applied = match std::pin::Pin::new(&mut job.handle).poll(&mut cx) {
                    std::task::Poll::Pending => return Ok(()),
                    std::task::Poll::Ready(result) => {
                        result.unwrap_or(Err(ControlBlock::ControlUnavailable))
                    }
                };
                let Some(job) = self.health.injecting.take() else {
                    return Ok(());
                };
                self.applied(job, applied).await?;
                // Releases that waited behind the injection go now.
                self.flush_releases();
            }
            let Some(next) = self.health.backlog.pop_front() else {
                // Drained: input queues in full again (round 10).
                self.health.overflow = false;
                return Ok(());
            };
            self.receipts_next();
            self.start_batch(&next).await?;
        }
    }

    /// `settle_input` on a capture tick, then the hold the viewer heard is
    /// checked against its reason now (round 12, S5: `on_frame` checks it
    /// after the new reference; without viewer frames only the ticks do).
    pub(in super::super) async fn settle_input_on_tick(&mut self) -> io::Result<()> {
        self.settle_input().await?;
        self.end_hold().await
    }

    /// What a finished injection means for the session and its viewer.
    pub(super) async fn applied(
        &mut self,
        job: super::super::input_release::Injection,
        applied: Result<(), ControlBlock>,
    ) -> io::Result<()> {
        // Round 12 (S1): of a failed batch, the calls that went in whole.
        let went: Vec<InputEvent> = job.went().cloned().collect();
        // Round 12 rest: its receipts (`host_input_receipts.rs`).
        let numbers = self.receipts_ended();
        let super::super::input_release::Injection {
            events,
            reference,
            presented,
            started,
            epoch,
            ..
        } = job;
        let events = &events[..];
        let sink = self.pipeline.input_sink();
        // Station `input_inject` (BK0): the injection of this batch.
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        crate::screen::core::chain_meter::lock(self.pipeline.chain_meter()).record_us(
            crate::screen::core::chain_meter::Station::InputInject,
            micros,
        );
        match applied {
            Ok(()) => {
                for event in events {
                    self.pressed.observe(event);
                }
                // Round 8 (I5): everything held was let go while this batch
                // went in; what it pressed goes too.
                if epoch != self.health.release_epoch {
                    self.release_pressed();
                }
                self.health.unconfirmed = None;
                // D6/E2: a block or hold the viewer heard ends with the
                // first batch that reached the host after it; a block the
                // platform reported late stays shown its minimum time (Z2).
                // Round 11 (N1): an overflowed backlog stays reported.
                let held = !self.health.overflow && self.health.held_reported.take().is_some();
                let late = self
                    .health
                    .block_shown_until
                    .is_some_and(|until| Instant::now() < until);
                let cleared = !late && self.health.block_reported.take().is_some();
                if cleared || held {
                    let state = ControlState {
                        holder: self.handle.controller(),
                        blocked: self.health.block_reported,
                        held: None,
                        input: None,
                    };
                    self.frame(HostFrame::ControlState(state)).await?;
                }
                self.receipts_applied(numbers, events).await
            }
            Err(blocked) => {
                for event in &went {
                    self.pressed.observe(event);
                }
                // Codex 3: the sink may keep releases of its own (Windows
                // lets go of what a partial batch pressed and keeps what it
                // refused); they are nudged until it confirms.
                self.health.sink_owes |= sink.is_some();
                // S3: a refusal for a reference the source just left holds
                // the batch back and keeps what the user holds; the new
                // reference lets go of it when it moves the floor.
                let stale = presented && self.stale_refusal(blocked, reference);
                if !went.is_empty() {
                    // Round 13 (S2): calls of its text went in before this
                    // one failed: the viewer hears how much was not typed and
                    // why, not that the whole batch was held back or blocked
                    // (a resend would type its start twice).
                    if stale {
                        self.release_held_ups(events);
                    } else {
                        self.release_pressed();
                    }
                    let (block, held) = if stale {
                        (None, Some(InputHeld::WaitingForPicture))
                    } else {
                        (Some(blocked), None)
                    };
                    self.stopped_partway(numbers.0, (events, &went), (block, held))
                        .await?;
                } else if stale {
                    // Held back, not blocked (review O2a); its releases of
                    // held keys go in (E2: the viewer sends each once).
                    self.release_held_ups(events);
                    let held = InputHeld::WaitingForPicture;
                    self.receipts_settled(numbers.0, InputResult::Held { held })
                        .await?;
                    return self.report_held(held).await;
                } else {
                    self.release_pressed();
                    self.receipts_settled(numbers.0, InputResult::Blocked { block: blocked })
                        .await?;
                    self.report_block(blocked).await?;
                }
                if !presented
                    && sink
                        .as_ref()
                        .is_some_and(|sink| sink.legacy_input_invalid())
                {
                    self.error(ScreenErrorCode::ControlUnavailable,
                        "Der Bildausschnitt hat sich geändert. Aufnahme läuft weiter; zum Steuern ist eine neue Verbindung erforderlich.".into()).await
                } else {
                    Ok(())
                }
            }
        }
    }
}
