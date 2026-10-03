//! Closes quiet peer connections while the device idles (K6) and keeps the
//! CPU awake while incoming peer streams run (K3).
//!
//! Every open QUIC connection sends a keepalive every five seconds, and each
//! one wakes the device. In low-power operation a sweep, run only when the
//! CPU is awake anyway (entering low power, server keepalive, alarm probe),
//! closes connections whose stream activity has not changed since a
//! snapshot at least two minutes (wall clock) old. Incoming connections
//! close from their own accept loop, so a stream that arrives at the same
//! moment is either served or provably never started (`IDLE`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use iroh::endpoint::{Connection, ConnectionError, ConnectionStats, VarInt};

use super::ShareIrohNode;
use crate::share::power::{PowerHub, STREAM_HOLD_MS};

/// Application close code of an idle close ("ID").
pub(crate) const IDLE_CLOSE_CODE: u32 = 0x4944;
pub(crate) const IDLE_CLOSE_REASON: &[u8] = b"idle";
/// A connection must stay quiet this long (wall clock) before it closes.
pub(crate) const IDLE_QUIET_MS: i64 = 120_000;
const STREAM_HOLD_RENEW: Duration = Duration::from_secs(30);

/// Whether the peer closed `connection` because it was idle: no stream on it
/// was ever accepted after the close decision, so a request that failed with
/// it was not executed and may be sent again on a new connection.
pub(crate) fn closed_idle(connection: &Connection) -> bool {
    matches!(
        connection.close_reason(),
        Some(ConnectionError::ApplicationClosed(close))
            if close.error_code == VarInt::from_u32(IDLE_CLOSE_CODE)
    )
}

/// Stream-level frames sent and received; QUIC keepalive pings and
/// acknowledgements leave it unchanged.
fn stream_activity(stats: &ConnectionStats) -> u64 {
    [&stats.frame_tx, &stats.frame_rx]
        .iter()
        .map(|frames| {
            frames
                .stream
                .saturating_add(frames.reset_stream)
                .saturating_add(frames.stop_sending)
                .saturating_add(frames.max_stream_data)
        })
        .fold(0u64, u64::saturating_add)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuietSnapshot {
    activity: u64,
    since_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SweepStep {
    Keep(QuietSnapshot),
    Close,
}

/// One connection of a sweep. `busy` connections (open incoming streams or
/// a bound mount lease) and changed activity restart the quiet period.
fn sweep_step(
    previous: Option<QuietSnapshot>,
    activity: u64,
    busy: bool,
    now_ms: i64,
) -> SweepStep {
    match previous {
        Some(snapshot)
            if !busy
                && snapshot.activity == activity
                && now_ms.saturating_sub(snapshot.since_ms) >= IDLE_QUIET_MS =>
        {
            SweepStep::Close
        }
        Some(snapshot) if !busy && snapshot.activity == activity && now_ms >= snapshot.since_ms => {
            SweepStep::Keep(snapshot)
        }
        _ => SweepStep::Keep(QuietSnapshot {
            activity,
            since_ms: now_ms,
        }),
    }
}

/// Per incoming filesystem connection: its open stream tasks, the mount
/// leases it used, and the close request of a sweep.
#[derive(Default)]
pub(crate) struct IncomingActivity {
    open_streams: AtomicUsize,
    leases: Mutex<HashSet<String>>,
    close: tokio::sync::Notify,
    yielding: AtomicBool,
}

impl IncomingActivity {
    pub(super) fn can_yield_connection(&self) -> bool {
        // Old lease tokens are tied to the physical connection. Preserve that
        // compatibility; current v2 acquisitions can reattach to the new one.
        !self
            .leases()
            .iter()
            .any(|token| !token.starts_with(crate::share::mount_lease::RELEASABLE_LEASE_PREFIX))
    }
    pub(in crate::share) fn fair_yield_requested(&self) -> bool {
        self.yielding.load(Ordering::Acquire)
    }
    pub(in crate::share) fn fair_yield_ready(&self) -> bool {
        self.open_streams.load(Ordering::Acquire) == 0
    }
    pub(super) fn request_fair_yield(&self) {
        self.yielding.store(true, Ordering::Release);
        self.close.notify_one();
    }
    fn stream_finished(&self) {
        if self.open_streams.fetch_sub(1, Ordering::AcqRel) == 1 && self.fair_yield_requested() {
            self.close.notify_one();
        }
    }
    fn leases(&self) -> MutexGuard<'_, HashSet<String>> {
        self.leases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A request on this connection used the authorized mount lease `token`:
    /// a mounted drive stays connected (documented limit of idle closing).
    pub(crate) fn lease_used(&self, token: &str) {
        let mut leases = self.leases();
        if !leases.contains(token) {
            leases.insert(token.to_string());
        }
    }

    pub(crate) fn lease_released(&self, token: &str) {
        self.leases().remove(token);
    }

    pub(crate) fn idle_close_allowed(&self) -> bool {
        self.open_streams.load(Ordering::Acquire) == 0 && self.leases().is_empty()
    }

    pub(crate) async fn close_requested(&self) {
        self.close.notified().await;
    }

    fn request_close(&self) {
        self.close.notify_one();
    }
}

/// An incoming connection as the node tracks it; Exec connections carry no
/// activity and are never closed by a sweep.
pub(super) struct IncomingEntry {
    pub(super) connection: Connection,
    pub(super) activity: Option<Arc<IncomingActivity>>,
}

/// Node-wide count of running incoming streams for the CPU hold.
#[derive(Default)]
pub(crate) struct StreamHolds {
    active: AtomicUsize,
    renewing: AtomicBool,
}

#[derive(Default)]
pub(super) struct NodeIdle {
    snapshots: Mutex<HashMap<usize, QuietSnapshot>>,
    pub(super) holds: Arc<StreamHolds>,
}

/// Keeps one incoming stream counted while its task runs.
struct IncomingStreamGuard {
    activity: Arc<IncomingActivity>,
    holds: Arc<StreamHolds>,
}

impl Drop for IncomingStreamGuard {
    fn drop(&mut self) {
        self.activity.stream_finished();
        self.holds.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct IdleSweepReport {
    pub(super) closed_outgoing: usize,
    pub(super) closing_incoming: usize,
}

impl ShareIrohNode {
    /// Closes quiet connections; the caller runs it only in low power.
    /// Returns how many connections it closed or asked to close.
    pub(crate) fn sweep_idle_connections(&self) -> usize {
        let report = self.sweep_idle_connections_at(self.power().now().wall_ms);
        report.closed_outgoing + report.closing_incoming
    }

    pub(super) fn sweep_idle_connections_at(&self, now_ms: i64) -> IdleSweepReport {
        let outgoing: Vec<(String, Connection)> = match self.sessions.lock() {
            Ok(sessions) => sessions
                .iter()
                .map(|(key, connection)| (key.clone(), connection.clone()))
                .collect(),
            Err(_) => Vec::new(),
        };
        let incoming: Vec<(Connection, Arc<IncomingActivity>)> = match self.incoming_sessions.lock()
        {
            Ok(entries) => entries
                .values()
                .filter_map(|entry| {
                    let activity = entry.activity.clone()?;
                    Some((entry.connection.clone(), activity))
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let mut snapshots = self
            .idle
            .snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut live = HashSet::new();
        let mut report = IdleSweepReport::default();
        for (key, connection) in outgoing {
            if connection.close_reason().is_some() {
                continue;
            }
            let id = connection.stable_id();
            let activity = stream_activity(&connection.stats());
            match sweep_step(snapshots.get(&id).copied(), activity, false, now_ms) {
                SweepStep::Keep(snapshot) => {
                    live.insert(id);
                    snapshots.insert(id, snapshot);
                }
                SweepStep::Close => {
                    // Only the exact cached generation; a replacement made
                    // meanwhile by a concurrent operation stays untouched.
                    if self.invalidate_outgoing_session(&key, id).unwrap_or(false) {
                        connection.close(VarInt::from_u32(IDLE_CLOSE_CODE), IDLE_CLOSE_REASON);
                        report.closed_outgoing += 1;
                    }
                }
            }
        }
        for (connection, activity) in incoming {
            if connection.close_reason().is_some() {
                continue;
            }
            let id = connection.stable_id();
            let busy = !activity.idle_close_allowed();
            let counters = stream_activity(&connection.stats());
            match sweep_step(snapshots.get(&id).copied(), counters, busy, now_ms) {
                SweepStep::Keep(snapshot) => {
                    live.insert(id);
                    snapshots.insert(id, snapshot);
                }
                SweepStep::Close => {
                    activity.request_close();
                    report.closing_incoming += 1;
                }
            }
        }
        snapshots.retain(|id, _| live.contains(id));
        report
    }

    /// An accepted stream prevents an idle-close race while its bounded
    /// frame is read. It has no CPU/power hold until authorization succeeds.
    pub(in crate::share) fn incoming_stream_pending(
        &self,
        activity: &Arc<IncomingActivity>,
    ) -> impl Send + 'static {
        activity.open_streams.fetch_add(1, Ordering::AcqRel);
        struct Pending(Arc<IncomingActivity>);
        impl Drop for Pending {
            fn drop(&mut self) {
                self.0.stream_finished();
            }
        }
        Pending(activity.clone())
    }

    /// Counts one incoming stream; in low power it also holds the CPU for
    /// `STREAM_HOLD_MS`, renewed every 30 s while any stream runs.
    pub(crate) fn incoming_stream_started(
        &self,
        activity: &Arc<IncomingActivity>,
    ) -> impl Send + 'static {
        activity.open_streams.fetch_add(1, Ordering::AcqRel);
        let holds = self.idle.holds.clone();
        holds.active.fetch_add(1, Ordering::AcqRel);
        let power = self.power().clone();
        power.request_hold(STREAM_HOLD_MS);
        if !holds.renewing.swap(true, Ordering::AcqRel) {
            // Only the shared counters and the hub: a task holding the node
            // could end up dropping its own runtime.
            self.rt.spawn(renew_stream_holds(holds.clone(), power));
        }
        IncomingStreamGuard {
            activity: activity.clone(),
            holds,
        }
    }
}

async fn renew_stream_holds(holds: Arc<StreamHolds>, power: Arc<PowerHub>) {
    loop {
        tokio::time::sleep(STREAM_HOLD_RENEW).await;
        if holds.active.load(Ordering::Acquire) == 0 {
            holds.renewing.store(false, Ordering::Release);
            // A stream that started between the check and the store found
            // `renewing` still set and spawned nothing; keep renewing for it.
            if holds.active.load(Ordering::Acquire) == 0
                || holds.renewing.swap(true, Ordering::AcqRel)
            {
                return;
            }
        }
        power.request_hold(STREAM_HOLD_MS);
    }
}

#[cfg(test)]
#[path = "node_idle_tests.rs"]
mod android_background_task_node_idle_tests;
