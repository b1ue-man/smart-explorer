//! Per-connection presence bundling for clients that negotiated
//! `idle_keepalive_v1`.
//!
//! The outbox remembers which presence route and lifetime this connection last
//! received per direct lookup and per room member. While the client is idle,
//! a pure refresh (same route) of such a presence is held back until the next
//! keepalive tick, provided the copy the client already holds stays valid at
//! least [`EXPIRY_MARGIN_SECS`] beyond that tick. First announcements, route
//! changes, refreshes of a nearly expired copy and everything that is not a
//! presence refresh are sent at once. Messages are handed to the writer queue
//! through `enqueue` while the caller holds the outbox lock, so deferred and
//! immediate messages to one client cannot overtake each other.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::time::{Duration, Instant};

use super::idle::Keepalive;
use super::limits::{
    MAX_ROOMS_PER_CLIENT, MAX_ROOM_MEMBERS, MAX_WATCHES_PER_CLIENT, MAX_WRITER_QUEUED_BYTES,
};
use super::{Out, PeerPresence};

/// Longest lifetime the server credits a presence with, counted from sending
/// it, whatever its signed `expires_at` claims.
const MAX_PRESENCE_LIFETIME_SECS: i64 = 300;
/// A deferred refresh leaves the client's copy valid at least this long
/// beyond the tick that delivers the refresh.
const EXPIRY_MARGIN_SECS: i64 = 30;
/// Deferred refreshes never retain more than one full writer queue; beyond it
/// refreshes are sent immediately and the normal queue bound applies.
const MAX_DEFERRED_BYTES: usize = MAX_WRITER_QUEUED_BYTES;
/// Most presences one client can legitimately hold at once. Keys beyond it
/// are not recorded, so their refreshes are simply never deferred.
const MAX_HELD_PRESENCES: usize = MAX_WATCHES_PER_CLIENT + MAX_ROOMS_PER_CLIENT * MAX_ROOM_MEMBERS;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum PresenceKey {
    Direct(String),
    Room { room_id: String, device_id: String },
}

impl PresenceKey {
    fn in_room(&self, room: &str) -> bool {
        matches!(self, Self::Room { room_id, .. } if room_id == room)
    }
}

/// What one connection holds for a key: route digest and wall-clock expiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HeldPresence {
    route: u64,
    expires_unix: i64,
}

struct Deferred {
    key: PresenceKey,
    held: HeldPresence,
    json: Vec<u8>,
}

struct IdleSchedule {
    keepalive: Keepalive,
    next_flush_at: Instant,
}

/// Idle state reported to the connection loop after a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IdleStatus {
    Active,
    Idle {
        next_flush_at: Instant,
        keepalive_sent: bool,
    },
}

pub(super) struct IdleOutbox {
    keepalive: Keepalive,
    schedule: Option<IdleSchedule>,
    held: HashMap<PresenceKey, HeldPresence>,
    deferred: Vec<Deferred>,
    deferred_bytes: usize,
    route_digest: RandomState,
}

impl IdleOutbox {
    pub(super) fn new(keepalive: Keepalive) -> Self {
        Self {
            keepalive,
            schedule: None,
            held: HashMap::new(),
            deferred: Vec::new(),
            deferred_bytes: 0,
            route_digest: RandomState::new(),
        }
    }

    /// Applies `set_idle` and enqueues its `idle_ack`. Leaving idle first
    /// delivers everything deferred.
    pub(super) fn set_idle(
        &mut self,
        idle: bool,
        proposal: Option<u32>,
        now: Instant,
        enqueue: &mut impl FnMut(Vec<u8>) -> bool,
    ) -> bool {
        let keepalive = if idle {
            let keepalive = self.keepalive.negotiate(proposal);
            let earliest = now + keepalive.duration();
            let next_flush_at = self
                .schedule
                .as_ref()
                .map_or(earliest, |schedule| schedule.next_flush_at.min(earliest));
            self.schedule = Some(IdleSchedule {
                keepalive,
                next_flush_at,
            });
            keepalive
        } else {
            self.schedule = None;
            self.flush(enqueue);
            self.keepalive
        };
        enqueue_message(
            &Out::IdleAck {
                idle,
                keepalive_secs: keepalive.secs(),
            },
            enqueue,
        )
    }

    /// Delivers deferred refreshes and `keepalive` once the tick is due.
    pub(super) fn tick(
        &mut self,
        now: Instant,
        enqueue: &mut impl FnMut(Vec<u8>) -> bool,
    ) -> IdleStatus {
        let Some(schedule) = self.schedule.as_mut() else {
            return IdleStatus::Active;
        };
        if now < schedule.next_flush_at {
            return IdleStatus::Idle {
                next_flush_at: schedule.next_flush_at,
                keepalive_sent: false,
            };
        }
        // Fixed cadence independent of inbound traffic; a late wake-up
        // delivers once and restarts the cadence from now.
        let period = schedule.keepalive.duration();
        schedule.next_flush_at += period;
        if schedule.next_flush_at <= now {
            schedule.next_flush_at = now + period;
        }
        let next_flush_at = schedule.next_flush_at;
        self.flush(enqueue);
        let keepalive_sent = enqueue_message(&Out::Keepalive, enqueue);
        IdleStatus::Idle {
            next_flush_at,
            keepalive_sent,
        }
    }

    /// Fan-out of a presence refresh: deferred while idle when allowed,
    /// otherwise sent like any other message.
    pub(super) fn offer(
        &mut self,
        message: &Out,
        json: Vec<u8>,
        now: Instant,
        unix: i64,
        enqueue: &mut impl FnMut(Vec<u8>) -> bool,
    ) -> bool {
        if let Some((key, held)) = self.presence_of(message, unix) {
            if self.may_defer(&key, held, json.len(), now, unix) {
                self.defer(key, held, json);
                return true;
            }
        }
        self.send(message, json, unix, enqueue)
    }

    /// Sends immediately and updates what the client holds.
    pub(super) fn send(
        &mut self,
        message: &Out,
        json: Vec<u8>,
        unix: i64,
        enqueue: &mut impl FnMut(Vec<u8>) -> bool,
    ) -> bool {
        let queued = enqueue(json);
        match message {
            Out::DirectAvailable { .. } | Out::RoomJoined { .. } => {
                if let (true, Some((key, held))) = (queued, self.presence_of(message, unix)) {
                    // The immediate copy supersedes an older deferred one.
                    self.remove_deferred(|deferred| deferred == &key);
                    self.hold(key, held);
                }
            }
            Out::RoomRoster { room_id, members } => {
                self.forget_room(room_id);
                if queued {
                    for member in members {
                        let held = self.held_presence(member, unix);
                        self.hold(room_key(room_id, &member.device_id), held);
                    }
                }
            }
            Out::DirectOffline { lookup_id } => {
                self.forget_key(&PresenceKey::Direct(lookup_id.clone()));
            }
            Out::RoomLeft { room_id, device_id } => {
                self.forget_key(&room_key(room_id, device_id));
            }
            _ => {}
        }
        queued
    }

    /// The client stopped watching `lookup_id`; a later watch starts fresh.
    pub(super) fn forget_direct(&mut self, lookup_id: &str) {
        self.forget_key(&PresenceKey::Direct(lookup_id.to_string()));
    }

    /// The client left `room_id`; a later join starts fresh.
    pub(super) fn forget_room(&mut self, room_id: &str) {
        self.held.retain(|key, _| !key.in_room(room_id));
        self.remove_deferred(|key| key.in_room(room_id));
    }

    #[cfg(test)]
    pub(super) fn deferred_len(&self) -> usize {
        self.deferred.len()
    }

    fn hold(&mut self, key: PresenceKey, held: HeldPresence) {
        if self.held.len() < MAX_HELD_PRESENCES || self.held.contains_key(&key) {
            self.held.insert(key, held);
        }
    }

    fn forget_key(&mut self, key: &PresenceKey) {
        self.held.remove(key);
        self.remove_deferred(|deferred| deferred == key);
    }

    fn may_defer(
        &self,
        key: &PresenceKey,
        candidate: HeldPresence,
        bytes: usize,
        now: Instant,
        unix: i64,
    ) -> bool {
        let Some(schedule) = &self.schedule else {
            return false;
        };
        // First presence for this key on this connection, or a new route.
        let Some(held) = self.held.get(key) else {
            return false;
        };
        if held.route != candidate.route {
            return false;
        }
        // Wall-clock instant of the next tick; judged on the wall clock so
        // the decision matches how the client checks the signed expiry.
        let until_flush = ceil_secs(schedule.next_flush_at.saturating_duration_since(now));
        let flush_unix = unix.saturating_add(until_flush);
        if held.expires_unix < flush_unix.saturating_add(EXPIRY_MARGIN_SECS) {
            return false;
        }
        let replaced = self
            .deferred
            .iter()
            .find(|deferred| &deferred.key == key)
            .map_or(0, |deferred| deferred.json.len());
        self.deferred_bytes - replaced + bytes <= MAX_DEFERRED_BYTES
    }

    fn defer(&mut self, key: PresenceKey, held: HeldPresence, json: Vec<u8>) {
        self.deferred_bytes += json.len();
        match self
            .deferred
            .iter_mut()
            .find(|deferred| deferred.key == key)
        {
            Some(existing) => {
                self.deferred_bytes -= existing.json.len();
                existing.held = held;
                existing.json = json;
            }
            None => self.deferred.push(Deferred { key, held, json }),
        }
    }

    fn flush(&mut self, enqueue: &mut impl FnMut(Vec<u8>) -> bool) {
        self.deferred_bytes = 0;
        for deferred in std::mem::take(&mut self.deferred) {
            // A refresh lost to a full queue leaves the older copy recorded.
            if enqueue(deferred.json) {
                self.hold(deferred.key, deferred.held);
            }
        }
    }

    fn remove_deferred(&mut self, matches: impl Fn(&PresenceKey) -> bool) {
        let mut removed = 0;
        self.deferred.retain(|deferred| {
            let remove = matches(&deferred.key);
            if remove {
                removed += deferred.json.len();
            }
            !remove
        });
        self.deferred_bytes -= removed;
    }

    fn presence_of(&self, message: &Out, unix: i64) -> Option<(PresenceKey, HeldPresence)> {
        match message {
            Out::DirectAvailable {
                lookup_id,
                presence,
            } => Some((
                PresenceKey::Direct(lookup_id.clone()),
                self.held_presence(presence, unix),
            )),
            Out::RoomJoined { room_id, presence } => Some((
                room_key(room_id, &presence.device_id),
                self.held_presence(presence, unix),
            )),
            _ => None,
        }
    }

    fn held_presence(&self, presence: &PeerPresence, unix: i64) -> HeldPresence {
        HeldPresence {
            route: self.route_digest.hash_one((
                &presence.node_id,
                &presence.relay_url,
                &presence.candidates,
            )),
            expires_unix: presence
                .expires_at
                .min(unix.saturating_add(MAX_PRESENCE_LIFETIME_SECS)),
        }
    }
}

fn room_key(room_id: &str, device_id: &str) -> PresenceKey {
    PresenceKey::Room {
        room_id: room_id.to_string(),
        device_id: device_id.to_string(),
    }
}

fn enqueue_message(message: &Out, enqueue: &mut impl FnMut(Vec<u8>) -> bool) -> bool {
    serde_json::to_vec(message).is_ok_and(enqueue)
}

fn ceil_secs(duration: Duration) -> i64 {
    let secs = duration.as_secs() + u64::from(duration.subsec_nanos() > 0);
    i64::try_from(secs).unwrap_or(i64::MAX)
}
