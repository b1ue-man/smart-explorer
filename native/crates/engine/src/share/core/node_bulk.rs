//! Bulk connections (Z-P6): up to `MAX_EXTRA` more QUIC connections to one
//! peer, used only for transfer streams (reads, single writes, packets),
//! once the first connection shows loss or a rate at which its one driver
//! task (packet building and AES-GCM on one core) becomes the limit. Each
//! connection gives the line its own congestion controller and driver.
//!
//! Every extra connection is made exactly like the first: hello, session
//! proof and the path admission of `connect_session_until` (LAN and direct
//! preferred; the relay as start bridge or marked fallback), bound to the current
//! authorization generation and kept in the node's session table under its
//! own key, so restrictions, idle sweeps and a stop close it like any other.
//! The host's admission stays per device (it counts transfers per
//! principal, not per connection), and the client's transfer slots do not
//! grow. Ranged uploads stay on the first connection: their stage table
//! lives there. A host without `bulk_connections_v1` gets one connection.
//! Relay payload never fans out (`route_start.rs`): while the first
//! connection runs over the relay no extra one is wanted or used, and an
//! extra connection still in its relay start phase carries no transfer.
//! An extra connection is made in the background (r-weg-4, 2026-11-07): the
//! transfer that asks for it never waits for its handshake and takes the
//! first connection until the extra one is cached and off the relay.
//! Plan 2026-11-14: every transfer stream gets priority −1 below the other
//! streams of the session (E4), and while an interactive session runs in
//! the process no further extra connection is added (E2: several
//! controllers together push harder than one).
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use iroh::endpoint::Connection;

use super::super::keepalive::{interactive_presence, transport_purpose};
use super::super::route_policy::route_start::carries_relay;
use super::{session_key, OpenedPeerStream, PeerEndpoint, ShareIdentity, ShareIrohNode};

/// Extra connections at most (four in all).
const MAX_EXTRA: usize = 3;
/// The first connection is measured at most this often.
const SAMPLE: Duration = Duration::from_secs(1);
/// A loss share above which more connections share the line, like
/// `iperf3 -P` (each controller backs off alone).
const LOSS_SHARE: f64 = 0.01;
/// Datagrams a sample needs before its loss counts.
const LOSS_MIN_DATAGRAMS: u64 = 1000;
/// Bytes per second (both directions) at which one connection's driver
/// nears the rate of one core (estimate, Z-P6). Plan 2026-11-14 E5/N1: set
/// to 80 % of the single-connection rate measured by M1 on the slowest
/// measured platform once that measurement exists.
const DRIVER_RATE: f64 = 250.0 * 1024.0 * 1024.0;
/// Time a background build of one extra connection may take (handshake,
/// hello, session proof and path admission).
const BUILD_BUDGET: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Bulk {
    sampled: Option<(Instant, u64, u64, u64)>,
    /// Extra connections this peer gets.
    wanted: usize,
    /// Round-robin position over the connections.
    next: usize,
    /// Extra connections (by turn) being made in the background.
    building: HashSet<usize>,
}

fn table() -> &'static Mutex<HashMap<String, Bulk>> {
    static TABLE: OnceLock<Mutex<HashMap<String, Bulk>>> = OnceLock::new();
    TABLE.get_or_init(Default::default)
}

fn extra_key(key: &str, index: usize) -> String {
    format!("{key}#bulk{index}")
}

/// Whether the first connection's last sample asks for one more extra
/// connection; updates the sample.
fn sample(bulk: &mut Bulk, connection: &Connection, now: Instant) -> bool {
    let stats = connection.stats();
    let bytes = stats.udp_tx.bytes.saturating_add(stats.udp_rx.bytes);
    let current = (now, bytes, stats.udp_tx.datagrams, stats.lost_packets);
    let Some((at, before_bytes, before_sent, before_lost)) = bulk.sampled else {
        bulk.sampled = Some(current);
        return false;
    };
    let elapsed = now.saturating_duration_since(at);
    if elapsed < SAMPLE {
        return false;
    }
    bulk.sampled = Some(current);
    let rate = bytes.saturating_sub(before_bytes) as f64 / elapsed.as_secs_f64();
    let sent = stats.udp_tx.datagrams.saturating_sub(before_sent);
    let lost = stats.lost_packets.saturating_sub(before_lost);
    let lossy = sent >= LOSS_MIN_DATAGRAMS && lost as f64 / sent as f64 >= LOSS_SHARE;
    lossy || rate >= DRIVER_RATE
}

/// Whether the peer gets one more extra connection: never above
/// `MAX_EXTRA`, never while an interactive session runs (`interactive`;
/// then the first connection is not even sampled), otherwise when `asks`
/// (the sample of the first connection) says so.
fn wants_more(wanted: usize, interactive: bool, asks: impl FnOnce() -> bool) -> bool {
    wanted < MAX_EXTRA && !interactive && asks()
}

/// The background build of extra connection `turn` under `key` ended.
fn built(key: &str, turn: usize) {
    if let Ok(mut table) = table().lock() {
        if let Some(bulk) = table.get_mut(key) {
            bulk.building.remove(&turn);
        }
    }
}

impl ShareIrohNode {
    /// A stream for one transfer to `endpoint`: on the first connection, or
    /// (when the host offers `bulk_connections_v1` and the first connection
    /// asks for it) on one of the extra ones in turn. An extra connection
    /// that cannot be made or used leaves the transfer on the first one.
    pub(in crate::share) fn open_transfer_stream_until(
        self: &Arc<Self>,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        deadline: Instant,
        bulk_offered: bool,
    ) -> io::Result<OpenedPeerStream> {
        let opened = self.open_transfer_stream_on(endpoint, identity, deadline, bulk_offered)?;
        transport_purpose::prioritize_transfer(&opened.send);
        Ok(opened)
    }

    /// The stream of `open_transfer_stream_until`, before its priority.
    fn open_transfer_stream_on(
        self: &Arc<Self>,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        deadline: Instant,
        bulk_offered: bool,
    ) -> io::Result<OpenedPeerStream> {
        if !bulk_offered {
            return self.open_stream_until(endpoint, identity, deadline);
        }
        let key = session_key(endpoint);
        let Some(primary) = self.healthy_cached_session(&key)? else {
            return self.open_stream_until(endpoint, identity, deadline);
        };
        if carries_relay(&primary) {
            // The relay server limits each client; more connections add load only.
            return self.open_stream_until(endpoint, identity, deadline);
        }
        let turn = match table().lock() {
            Ok(mut table) => {
                let bulk = table.entry(key.clone()).or_default();
                let interactive = interactive_presence::active();
                if wants_more(bulk.wanted, interactive, || {
                    sample(bulk, &primary, Instant::now())
                }) {
                    bulk.wanted += 1;
                }
                bulk.next = bulk.next.wrapping_add(1);
                bulk.next % (bulk.wanted + 1)
            }
            Err(_) => 0,
        };
        if turn == 0 {
            return self.open_stream_until(endpoint, identity, deadline);
        }
        let extra = extra_key(&key, turn);
        let Some(connection) = self.healthy_cached_session(&extra)? else {
            self.build_extra_in_background(endpoint, identity, &key, turn);
            return self.open_stream_until(endpoint, identity, deadline);
        };
        if carries_relay(&connection) {
            // Still in its start phase while the first connection is direct:
            // this transfer stays direct; the extra one keeps establishing its
            // direct path for later turns.
            return self.open_stream_until(endpoint, identity, deadline);
        }
        match self.open_on_connection(&extra, &connection, deadline, "bulk") {
            Ok(opened) => Ok(opened),
            Err(_) => {
                // Nothing was sent yet: this connection is dropped.
                let _ = self.invalidate_outgoing_session(&extra, connection.stable_id());
                self.open_stream_until(endpoint, identity, deadline)
            }
        }
    }

    /// Starts making the extra connection `turn` of the peer under `key`
    /// on its own thread, unless one is being made already. A failure only
    /// leaves the transfers on the first connection; a later turn tries
    /// again.
    fn build_extra_in_background(
        self: &Arc<Self>,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        key: &str,
        turn: usize,
    ) {
        let started = match table().lock() {
            Ok(mut table) => table
                .entry(key.to_string())
                .or_default()
                .building
                .insert(turn),
            Err(_) => false,
        };
        if !started {
            return;
        }
        let node = Arc::clone(self);
        let endpoint = endpoint.clone();
        let identity = identity.clone();
        let owner = key.to_string();
        let spawned = std::thread::Builder::new()
            .name("se-bulk-connect".into())
            .spawn(move || {
                let extra = extra_key(&owner, turn);
                let deadline = Instant::now() + BUILD_BUDGET;
                let _ = node.extra_connection(&endpoint, &identity, deadline, &extra);
                built(&owner, turn);
            });
        if spawned.is_err() {
            built(key, turn);
        }
    }

    /// The extra connection `extra`, made like the first one when it is
    /// missing or closed.
    fn extra_connection(
        &self,
        endpoint: &PeerEndpoint,
        identity: &ShareIdentity,
        deadline: Instant,
        extra: &str,
    ) -> io::Result<Connection> {
        if let Some(connection) = self.healthy_cached_session(extra)? {
            return Ok(connection);
        }
        let principal = super::super::session::PeerPrincipal::from_endpoint(endpoint);
        let generation = self.policy.snapshot(principal)?;
        generation.check()?;
        let connection = self.connect_session_until(endpoint, identity, deadline)?;
        self.cache_session(extra.to_string(), connection.clone(), generation)?;
        Ok(connection)
    }

    /// Extra connections the peer `endpoint` gets from now on (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::share) fn want_bulk_connections(&self, endpoint: &PeerEndpoint, extra: usize) {
        if let Ok(mut table) = table().lock() {
            table.entry(session_key(endpoint)).or_default().wanted = extra.min(MAX_EXTRA);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbindungen_task_bulk_no_growth_with_presence() {
        // An interactive session stops growth without sampling.
        assert!(!wants_more(0, true, || panic!(
            "no sample while interactive"
        )));
        assert!(!wants_more(MAX_EXTRA - 1, true, || true));
        // Without one, the sample decides as before, up to MAX_EXTRA.
        assert!(wants_more(0, false, || true));
        assert!(!wants_more(0, false, || false));
        assert!(!wants_more(MAX_EXTRA, false, || panic!(
            "no sample at the cap"
        )));
        assert_eq!(transport_purpose::TRANSFER_STREAM_PRIORITY, -1);
    }
}
