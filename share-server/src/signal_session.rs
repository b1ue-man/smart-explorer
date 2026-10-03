//! Liveness timing of one registered signaling connection, shared by the TCP
//! and WebSocket loops.
//!
//! Not idle: both transports close after [`ACTIVE_READ_WINDOW`] without
//! inbound data. Idle (both transports): no inbound data is required between
//! keepalive ticks; after each `keepalive` the client must send something
//! within [`IDLE_REPLY_WINDOW`].
//!
//! [`ACTIVE_READ_WINDOW`]: super::idle::ACTIVE_READ_WINDOW

use std::time::{Duration, Instant};

use super::idle::IDLE_REPLY_WINDOW;
use super::idle_outbox::IdleStatus;
use super::Writer;

/// Next action of a connection loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Read until this instant at most (`None`: no deadline), then poll again.
    Wait(Option<Instant>),
    Close,
}

pub(super) struct SignalSession {
    /// Inbound silence that closes a connection that is not idle; `None`
    /// keeps it open indefinitely.
    active_window: Option<Duration>,
    last_inbound: Instant,
    reply_deadline: Option<Instant>,
}

impl SignalSession {
    pub(super) fn new(now: Instant, active_window: Option<Duration>) -> Self {
        Self {
            active_window,
            last_inbound: now,
            reply_deadline: None,
        }
    }

    /// Any inbound bytes or frames prove the client is alive.
    pub(super) fn inbound(&mut self, now: Instant) {
        self.last_inbound = now;
        self.reply_deadline = None;
    }

    /// Runs a due keepalive tick and returns how long the loop may read.
    pub(super) fn poll(&mut self, writer: &Writer, now: Instant) -> Step {
        if self.reply_deadline.is_some_and(|deadline| now >= deadline) {
            return Step::Close;
        }
        match writer.idle_tick(now) {
            IdleStatus::Active => {
                self.reply_deadline = None;
                let Some(window) = self.active_window else {
                    return Step::Wait(None);
                };
                let deadline = self.last_inbound + window;
                if now >= deadline {
                    Step::Close
                } else {
                    Step::Wait(Some(deadline))
                }
            }
            IdleStatus::Idle {
                next_flush_at,
                keepalive_sent,
            } => {
                if keepalive_sent && self.reply_deadline.is_none() {
                    self.reply_deadline = Some(now + IDLE_REPLY_WINDOW);
                }
                let until = self
                    .reply_deadline
                    .map_or(next_flush_at, |deadline| deadline.min(next_flush_at));
                Step::Wait(Some(until))
            }
        }
    }
}
