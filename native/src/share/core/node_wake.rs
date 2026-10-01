//! Wakes the signal worker of a node: its own stop, power changes and
//! probes (via the power hub), and every change of the endpoint's published
//! routes or home-relay state, so the worker waits for events instead of
//! polling its connection.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};
use iroh::{Endpoint, Watcher as _};

use super::ShareIrohNode;
use crate::share::power::PowerHub;

/// Network-change notifications closer together than this are one change
/// (Android reports a new default network in several callbacks).
pub(super) const NETWORK_CHANGE_DEBOUNCE: Duration = Duration::from_secs(2);

pub(super) struct NodeWake {
    sender: Sender<()>,
    receiver: Receiver<()>,
    routes: Arc<AtomicU64>,
    last_network_change: Mutex<Option<Instant>>,
}

impl NodeWake {
    pub(super) fn new() -> Self {
        // One pending wake is enough: the worker re-reads all state.
        let (sender, receiver) = bounded(1);
        Self {
            sender,
            receiver,
            routes: Arc::new(AtomicU64::new(0)),
            last_network_change: Mutex::new(None),
        }
    }

    pub(super) fn wake(&self) {
        let _ = self.sender.try_send(());
    }

    pub(super) fn sender(&self) -> Sender<()> {
        self.sender.clone()
    }

    pub(super) fn receiver(&self) -> &Receiver<()> {
        &self.receiver
    }

    /// Changes whenever the published routes or the home-relay state may
    /// have changed; the worker republishes its presence on every change.
    pub(super) fn route_revision(&self) -> u64 {
        self.routes.load(Ordering::Acquire)
    }

    pub(super) fn watch(&self, runtime: &tokio::runtime::Runtime, endpoint: &Endpoint) {
        let mut addresses = endpoint.watch_addr();
        let _ = addresses.get();
        let (routes, sender) = (self.routes.clone(), self.sender.clone());
        runtime.spawn(async move {
            while addresses.updated().await.is_ok() {
                routes.fetch_add(1, Ordering::AcqRel);
                let _ = sender.try_send(());
            }
        });
        // A home relay selected before its handshake completed becomes usable
        // later; a lost relay must be noticed without polling. Only the
        // connected state counts: each failed reconnect changes the recorded
        // error, and republishing on every one would defeat idle operation.
        let mut relays = endpoint.home_relay_status();
        let mut connected = any_connected(&relays.get());
        let (routes, sender) = (self.routes.clone(), self.sender.clone());
        runtime.spawn(async move {
            while let Ok(status) = relays.updated().await {
                let now_connected = any_connected(&status);
                if now_connected != connected {
                    connected = now_connected;
                    routes.fetch_add(1, Ordering::AcqRel);
                    let _ = sender.try_send(());
                }
            }
        });
    }

    /// Whether a network change at `now` is new enough to be forwarded.
    pub(super) fn network_change_due(&self, now: Instant) -> bool {
        let mut last = self
            .last_network_change
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.is_some_and(|last| now.saturating_duration_since(last) < NETWORK_CHANGE_DEBOUNCE) {
            return false;
        }
        *last = Some(now);
        true
    }
}

impl ShareIrohNode {
    pub(crate) fn power(&self) -> &Arc<PowerHub> {
        &self.power
    }

    /// Wakes this node's signal worker (stop, power change, routes).
    pub(crate) fn wake_signal_worker(&self) {
        self.wake.wake();
    }

    pub(crate) fn signal_wake_sender(&self) -> Sender<()> {
        self.wake.sender()
    }

    pub(crate) fn signal_wake(&self) -> &Receiver<()> {
        self.wake.receiver()
    }

    /// Forwards a host-reported network change to Iroh (rebinds sockets,
    /// re-probes the relay), at most once per debounce interval.
    pub(crate) fn notify_network_change(&self) -> bool {
        if !self.wake.network_change_due(Instant::now()) {
            return false;
        }
        self.block_on(self.endpoint.network_change());
        true
    }

    /// `None` without a configured relay, else whether a home relay is
    /// connected.
    pub(crate) fn home_relay_connected(&self) -> Option<bool> {
        self.relay_configured
            .then(|| any_connected(&self.endpoint.home_relay_status().get()))
    }
}

fn any_connected(status: &[iroh::endpoint::RelayStatus]) -> bool {
    status.iter().any(|relay| relay.is_connected())
}
