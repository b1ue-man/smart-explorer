//! The peers' freshly verified presences and on-demand presence refresh
//! rounds (user report 2026-11-25: „das aktuallisieren der ip adresse muss
//! automatisch laufen beim verbindungsaufbau“).
//!
//! The signal worker records every presence it verified from the Share
//! server here at once (`tracked_signal_dispatch.rs`), before the daemon has
//! stored it in the profiles, so a connection setup dials the addresses a
//! peer published last (`peer_dial.rs`). Within one signal session the
//! server pushes every republish of a watched peer; a cached presence can
//! only be stale from before the current session (pushes missed while the
//! session was down, for example after a network change). A setup therefore
//! asks once per peer and session for a refresh round: the worker sends
//! `watch_direct` again for the peer's Direct lookups (the server answers
//! with the presence it holds) and then a heartbeat; the server answers lines
//! in order, so the next pong completes the round. The setup waits for it at
//! most [`REFRESH_WAIT`] and otherwise dials the cached addresses. Without a
//! connected session it does not wait, and after one round timed out the
//! rest of the session does not wait either until a pong arrives (no
//! repeated delays while the server is silent).
//!
//! Every change (a recorded presence, a completed round, a session start or
//! end, a change of this device's addresses, a relation runtime update)
//! advances one watch counter; it wakes the direct retries of the route
//! actors (`route_monitor_retry.rs`).
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::{Endpoint, Watcher as _};
use tokio::sync::watch;

use crate::share::core::now_secs;
use crate::share::types::{PeerPresence, ShareEvent};

/// Longest wait of a connection setup for a refresh round.
pub(in crate::share) const REFRESH_WAIT: Duration = Duration::from_millis(1_500);
/// Presences kept (relation and device each).
const MAX_PRESENCES: usize = 512;
/// Peers one round may still wait for.
const MAX_WANTED: usize = 256;

/// (kind, relation id, node id) of a presence.
type PresenceKey = (String, String, String);

pub(in crate::share) struct PeerAddressBook {
    state: Mutex<BookState>,
    changes: watch::Sender<u64>,
    /// Wakes the signal worker to send a round.
    wake: crossbeam_channel::Sender<()>,
}

#[derive(Default)]
struct BookState {
    fresh: HashMap<PresenceKey, PeerPresence>,
    connected: bool,
    session: u64,
    /// A round of this session timed out; no wait until the next pong.
    stalled: bool,
    /// Node id → session in which a round confirmed its presence.
    confirmed: HashMap<String, u64>,
    wanted: BTreeSet<String>,
    in_flight: Option<(u64, Vec<String>)>,
    next_round: u64,
    completed: u64,
}

impl PeerAddressBook {
    pub(in crate::share) fn new(wake: crossbeam_channel::Sender<()>) -> Self {
        let (changes, _) = watch::channel(0);
        Self {
            state: Mutex::new(BookState {
                next_round: 1,
                ..BookState::default()
            }),
            changes,
            wake,
        }
    }

    /// A book whose counter also follows this device's published addresses.
    pub(in crate::share) fn start(
        runtime: &tokio::runtime::Runtime,
        endpoint: &Endpoint,
        wake: crossbeam_channel::Sender<()>,
    ) -> Arc<Self> {
        let book = Arc::new(Self::new(wake));
        let mut addresses = endpoint.watch_addr();
        let _ = addresses.get();
        let changes = book.changes.clone();
        runtime.spawn(async move {
            while addresses.updated().await.is_ok() {
                changes.send_modify(advance);
            }
        });
        book
    }

    pub(in crate::share) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Addresses may have changed (relation runtime update).
    pub(in crate::share) fn changed(&self) {
        self.changes.send_modify(advance);
    }

    /// Records the presences of a worker event that passed verification.
    pub(in crate::share) fn record_event(&self, event: &ShareEvent) {
        match event {
            ShareEvent::DirectAvailable { presence, .. }
            | ShareEvent::RoomJoined { presence, .. } => self.record(presence),
            ShareEvent::RoomRoster { members, .. } => members.iter().for_each(|p| self.record(p)),
            _ => {}
        }
    }

    /// Keeps `presence` unless a later one of the same relation and device is known.
    pub(in crate::share) fn record(&self, presence: &PeerPresence) {
        let now = now_secs();
        if !presence.is_current_at(now) {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let key = key_of(&presence.kind, &presence.relation_id, &presence.node_id);
        if state
            .fresh
            .get(&key)
            .is_some_and(|known| known.expires_at > presence.expires_at)
        {
            return;
        }
        if !state.fresh.contains_key(&key) && state.fresh.len() >= MAX_PRESENCES {
            state.fresh.retain(|_, known| known.is_current_at(now));
            let oldest = state
                .fresh
                .iter()
                .min_by_key(|(_, known)| known.expires_at)
                .map(|(key, _)| key.clone());
            if let (true, Some(oldest)) = (state.fresh.len() >= MAX_PRESENCES, oldest) {
                state.fresh.remove(&oldest);
            }
        }
        state.fresh.insert(key, presence.clone());
        drop(state);
        self.changed();
    }

    /// The newest current verified presence of `kind`/`relation_id`/`node_id`.
    pub(in crate::share) fn fresh_for(
        &self,
        kind: &str,
        relation_id: &str,
        node_id: &str,
    ) -> Option<PeerPresence> {
        let state = self.state.lock().ok()?;
        state
            .fresh
            .get(&key_of(kind, relation_id, node_id))
            .filter(|presence| presence.is_current_at(now_secs()))
            .cloned()
    }

    /// [`Self::fresh_for`] for the relation and device of `presence`.
    pub(in crate::share) fn fresh(&self, presence: &PeerPresence) -> Option<PeerPresence> {
        self.fresh_for(&presence.kind, &presence.relation_id, &presence.node_id)
    }

    /// Setup side: the round that refreshes `node_id`, or `None` when the
    /// setup must not wait (no session, a silent server, already refreshed in
    /// this session).
    pub(in crate::share) fn request(&self, node_id: &str) -> Option<u64> {
        let mut state = self.state.lock().ok()?;
        if !state.connected
            || state.stalled
            || state.confirmed.get(node_id) == Some(&state.session)
            || (state.wanted.len() >= MAX_WANTED && !state.wanted.contains(node_id))
        {
            return None;
        }
        state.wanted.insert(node_id.to_string());
        let round = state.next_round;
        drop(state);
        let _ = self.wake.try_send(());
        Some(round)
    }

    /// Waits at most `bound` for `round`; returns whether it completed.
    pub(in crate::share) async fn wait_round(&self, round: u64, bound: Duration) -> bool {
        let mut changes = self.changes.subscribe();
        let completed = async {
            loop {
                if self.completed(round) {
                    return true;
                }
                if changes.changed().await.is_err() {
                    return false;
                }
            }
        };
        match tokio::time::timeout(bound, completed).await {
            Ok(done) => done,
            Err(_) => {
                if let Ok(mut state) = self.state.lock() {
                    state.stalled = state.connected;
                }
                false
            }
        }
    }

    fn completed(&self, round: u64) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.completed >= round)
    }

    /// Worker side: a signal session serves from now on.
    pub(in crate::share) fn session_started(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.connected = true;
            state.session = state.session.wrapping_add(1);
            state.stalled = false;
            state.confirmed.clear();
        }
        self.changed();
    }

    /// Worker side: the session ended; an unanswered round goes to the next one.
    pub(in crate::share) fn session_ended(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.connected = false;
            if let Some((_, nodes)) = state.in_flight.take() {
                state.wanted.extend(nodes);
            }
        }
        self.changed();
    }

    /// Worker side: the peers of the next round, now in flight.
    pub(in crate::share) fn take_round(&self) -> Option<Vec<String>> {
        let mut state = self.state.lock().ok()?;
        if !state.connected || state.in_flight.is_some() || state.wanted.is_empty() {
            return None;
        }
        let nodes: Vec<String> = std::mem::take(&mut state.wanted).into_iter().collect();
        let round = state.next_round;
        state.next_round = round.wrapping_add(1);
        state.in_flight = Some((round, nodes.clone()));
        Some(nodes)
    }

    /// Worker side: a pong arrived; it completes the round in flight.
    pub(in crate::share) fn pong(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.stalled = false;
        let Some((round, nodes)) = state.in_flight.take() else {
            return;
        };
        state.completed = state.completed.max(round);
        let session = state.session;
        if state.confirmed.len() >= MAX_PRESENCES {
            // Bounded: the peers simply ask once more in this session.
            state.confirmed.clear();
        }
        for node in nodes {
            state.confirmed.insert(node, session);
        }
        drop(state);
        self.changed();
    }
}

fn key_of(kind: &str, relation_id: &str, node_id: &str) -> PresenceKey {
    (kind.into(), relation_id.into(), node_id.into())
}

fn advance(counter: &mut u64) {
    *counter = counter.wrapping_add(1);
}
