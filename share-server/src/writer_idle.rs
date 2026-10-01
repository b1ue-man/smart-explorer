//! Idle-keepalive side of [`Writer`]: presence bundling and the keepalive
//! tick. Every enqueue happens under the outbox lock, so deferred and
//! immediate messages to one client keep their order.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use super::Writer;
use crate::idle::{SignalClock, SignalTiming};
use crate::idle_outbox::{IdleOutbox, IdleStatus};
use crate::Out;

pub(super) struct IdleLink {
    clock: Arc<dyn SignalClock>,
    outbox: Mutex<IdleOutbox>,
}

impl IdleLink {
    fn lock(&self) -> MutexGuard<'_, IdleOutbox> {
        self.outbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn send(
        &self,
        message: &Out,
        json: Vec<u8>,
        enqueue: &mut impl FnMut(Vec<u8>) -> bool,
    ) -> bool {
        let unix = self.clock.unix_secs();
        self.lock().send(message, json, unix, enqueue)
    }
}

impl Writer {
    /// Enables presence bundling and idle keepalive for a client that
    /// negotiated `idle_keepalive_v1`. Later calls have no effect.
    pub(crate) fn enable_idle(&self, timing: &SignalTiming) {
        let _ = self.idle.set(IdleLink {
            clock: Arc::clone(&timing.clock),
            outbox: Mutex::new(IdleOutbox::new(timing.keepalive)),
        });
    }

    /// Fan-out of a presence refresh (`direct_available`, `room_joined`):
    /// an idle client may receive it with its next keepalive.
    pub(crate) fn offer(&self, message: &Out) -> bool {
        let Some(link) = self.idle.get() else {
            return self.try_send(message);
        };
        let Some(json) = self.serialize(message) else {
            return false;
        };
        let (now, unix) = (link.clock.now(), link.clock.unix_secs());
        link.lock().offer(message, json, now, unix, &mut |json| {
            self.enqueue_json(json)
        })
    }

    /// Handles `set_idle`; ignored unless the capability was negotiated.
    pub(crate) fn set_idle(&self, idle: bool, keepalive_secs: Option<u32>) {
        let Some(link) = self.idle.get() else {
            return;
        };
        let now = link.clock.now();
        link.lock()
            .set_idle(idle, keepalive_secs, now, &mut |json| {
                self.enqueue_json(json)
            });
    }

    /// Runs a due keepalive tick and reports the idle schedule.
    pub(crate) fn idle_tick(&self, now: Instant) -> IdleStatus {
        match self.idle.get() {
            Some(link) => link.lock().tick(now, &mut |json| self.enqueue_json(json)),
            None => IdleStatus::Active,
        }
    }

    /// The client unwatched `lookup_id`.
    pub(crate) fn forget_idle_direct(&self, lookup_id: &str) {
        if let Some(link) = self.idle.get() {
            link.lock().forget_direct(lookup_id);
        }
    }

    /// The client left `room_id`.
    pub(crate) fn forget_idle_room(&self, room_id: &str) {
        if let Some(link) = self.idle.get() {
            link.lock().forget_room(room_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn idle_deferred_len(&self) -> usize {
        self.idle.get().map_or(0, |link| link.lock().deferred_len())
    }
}
