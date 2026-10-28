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
use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use iroh::endpoint::Connection;

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
/// nears the rate of one core (estimate, Z-P6).
const DRIVER_RATE: f64 = 250.0 * 1024.0 * 1024.0;

#[derive(Default)]
struct Bulk {
    sampled: Option<(Instant, u64, u64, u64)>,
    /// Extra connections this peer gets.
    wanted: usize,
    /// Round-robin position over the connections.
    next: usize,
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

impl ShareIrohNode {
    /// A stream for one transfer to `endpoint`: on the first connection, or
    /// (when the host offers `bulk_connections_v1` and the first connection
    /// asks for it) on one of the extra ones in turn. An extra connection
    /// that cannot be made or used leaves the transfer on the first one.
    pub(in crate::share) fn open_transfer_stream_until(
        &self,
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
                if bulk.wanted < MAX_EXTRA && sample(bulk, &primary, Instant::now()) {
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
        let opened = self
            .extra_connection(endpoint, identity, deadline, &extra)
            .and_then(|connection| {
                if carries_relay(&connection) {
                    // Still in its start phase while the first connection is
                    // direct: this transfer stays direct; the extra one keeps
                    // establishing its direct path for later turns.
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "Zusatzverbindung noch über Relay",
                    ));
                }
                let opened = self.open_on_connection(&extra, &connection, deadline, "bulk");
                if opened.is_err() {
                    // Nothing was sent yet: this connection is dropped.
                    let _ = self.invalidate_outgoing_session(&extra, connection.stable_id());
                }
                opened
            });
        match opened {
            Ok(opened) => Ok(opened),
            Err(_) => self.open_stream_until(endpoint, identity, deadline),
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
    #[cfg(test)]
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
    fn speed_task_share_bulk_keys_are_distinct_per_connection() {
        assert_eq!(extra_key("direct:r:n", 1), "direct:r:n#bulk1");
        assert_ne!(extra_key("direct:r:n", 1), extra_key("direct:r:n", 2));
    }
}
