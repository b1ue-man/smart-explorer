//! Writes the QUIC statistics of Share connections into a file the speed
//! stage collects (diagnosis, E13). One task per node reads every tracked
//! connection twice a second, lets a `Tracker` decide when a transfer burst
//! begins and ends (`quic_stats.rs`) and appends one JSON line per finished
//! burst to `share-quic-stats.jsonl` in the application data folder of the
//! process.
//!
//! This is a measurement aid of development builds (`transfer_meter::enabled`):
//! no setting, no interface, no protocol byte, and a release build or a
//! process with `SE_TEST_NO_SHARE_STATS` set tracks nothing. Both ends write:
//! the opening side as `client`, the accepting side as `host`, so a line of
//! each side exists for the same transfer. The speed stage reads the file of
//! the profile it runs in and, from the path next to the export, the file of
//! the other device.
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use iroh::endpoint::{Connection, WeakConnectionHandle};
use serde::Serialize;
use tokio::time::MissedTickBehavior;

use super::keepalive::{transport_windows, QUIC_CC_VARIABLE};
use super::node::ShareIrohNode;
use super::quic_stats::{Ambient, PathKind, Report, Sample, Tracker, TICK};
use super::quic_stats_proc::{
    busiest_threads, read_buffer_limits, read_process_cpu_ms, read_thread_cpu, read_udp_counters,
    BufferLimits, ThreadCpu,
};
use super::transfer_meter::{enabled, meter};

/// The file of the lines, in the application data folder.
pub(super) const STATS_FILE: &str = "share-quic-stats.jsonl";
/// A file above this size is replaced by an empty one (the old content stays
/// in `<name>.1`), so a long-running development build cannot fill a disk.
pub(super) const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Connections tracked at once; the oldest closed ones go first.
const MAX_TRACKED: usize = 256;
/// Threads named in a line.
const BUSIEST_THREADS: usize = 4;

/// Which end of the connection wrote the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    /// The device that opened the connection (and its streams).
    Client,
    /// The device that accepted it.
    Host,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Side::Client => "client",
            Side::Host => "host",
        }
    }
}

struct Tracked {
    id: usize,
    side: Side,
    handle: WeakConnectionHandle,
}

#[derive(Default)]
struct Shared {
    tracked: Mutex<Vec<Tracked>>,
}

impl Shared {
    /// The tracked connections that are still open; closed or dropped ones
    /// leave the list.
    fn live(&self) -> Vec<(usize, Side, Connection)> {
        let Ok(mut tracked) = self.tracked.lock() else {
            return Vec::new();
        };
        let mut live = Vec::with_capacity(tracked.len());
        tracked.retain(|entry| match entry.handle.upgrade() {
            Some(connection) if connection.close_reason().is_none() => {
                live.push((entry.id, entry.side, connection));
                true
            }
            _ => false,
        });
        live
    }
}

/// The connections of a node whose statistics are written.
#[derive(Clone, Default)]
pub(super) struct QuicStatsHub {
    shared: Arc<Shared>,
}

impl QuicStatsHub {
    /// A hub whose reporter runs on `rt` where measuring is on.
    pub(super) fn start(rt: &tokio::runtime::Runtime) -> Self {
        let hub = Self::default();
        if enabled() {
            rt.spawn(report(hub.shared.clone()));
        }
        hub
    }

    /// Follows `connection` from now on (once per connection).
    pub(super) fn track(&self, connection: &Connection, side: Side) {
        if !enabled() {
            return;
        }
        let id = connection.stable_id();
        let Ok(mut tracked) = self.shared.tracked.lock() else {
            return;
        };
        if tracked.iter().any(|entry| entry.id == id) || tracked.len() >= MAX_TRACKED {
            return;
        }
        tracked.push(Tracked {
            id,
            side,
            handle: connection.weak_handle(),
        });
    }
}

/// Follows the outgoing connection `session_key` of `node` (a stream is about
/// to use it).
pub(super) fn track_client(node: &ShareIrohNode, session_key: &str) {
    if !enabled() {
        return;
    }
    let connection = node
        .sessions
        .lock()
        .ok()
        .and_then(|sessions| sessions.get(session_key).cloned());
    if let Some(connection) = connection {
        node.quic_stats.track(&connection, Side::Client);
    }
}

/// What the reporter keeps per connection.
struct Entry {
    side: Side,
    tracker: Tracker,
    threads_at_start: Vec<ThreadCpu>,
}

/// The counters of one connection now.
fn read_sample(connection: &Connection) -> Sample {
    let stats = connection.stats();
    let paths = connection.paths();
    let selected = paths
        .iter()
        .find(|path| path.is_selected())
        .or_else(|| paths.iter().next());
    let (path_stats, kind) = match &selected {
        Some(path) => (
            path.stats(),
            if path.is_ip() {
                PathKind::Ip
            } else {
                PathKind::Relay
            },
        ),
        None => (Default::default(), PathKind::None),
    };
    Sample::from_noq(&stats, &path_stats, kind, paths.len())
}

fn read_ambient() -> Ambient {
    Ambient {
        meter: meter().snapshot(),
        cpu_ms: read_process_cpu_ms(),
        udp: read_udp_counters(),
    }
}

async fn report(shared: Arc<Shared>) {
    let origin = Instant::now();
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut entries: HashMap<usize, Entry> = HashMap::new();
    loop {
        ticks.tick().await;
        // The node (and with it the hub) is gone: only this task is left.
        let alone = Arc::strong_count(&shared) == 1;
        let live = if alone { Vec::new() } else { shared.live() };
        let now_ms = u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        if !live.is_empty() {
            let ambient = read_ambient();
            for (id, side, connection) in &live {
                let entry = entries.entry(*id).or_insert_with(|| Entry {
                    side: *side,
                    tracker: Tracker::default(),
                    threads_at_start: Vec::new(),
                });
                let step = entry
                    .tracker
                    .observe(now_ms, read_sample(connection), ambient);
                if step.started {
                    entry.threads_at_start = read_thread_cpu();
                }
                if let Some(burst) = step.report {
                    write_burst(*id, entry, &burst);
                }
            }
        }
        let gone: Vec<usize> = entries
            .keys()
            .filter(|id| !live.iter().any(|(live_id, _, _)| live_id == *id))
            .copied()
            .collect();
        for id in gone {
            if let Some(mut entry) = entries.remove(&id) {
                if let Some(burst) = entry.tracker.finish() {
                    write_burst(id, &entry, &burst);
                }
            }
        }
        if alone {
            return;
        }
    }
}

/// The congestion controller this process was started with.
fn controller_name() -> &'static str {
    match std::env::var(QUIC_CC_VARIABLE).as_deref() {
        Ok("bbr3") => "bbr3",
        _ => "cubic",
    }
}

#[derive(Serialize)]
struct Windows {
    stream_bytes: u32,
    connection_bytes: u32,
    send_bytes: u64,
}

/// One line of the file: a finished burst of one connection.
#[derive(Serialize)]
struct Line<'a> {
    kind: &'static str,
    ts_ms: u64,
    pid: u32,
    side: &'static str,
    conn: usize,
    seq: u64,
    cc: &'static str,
    windows: Windows,
    socket_buffers: BufferLimits,
    busiest_threads: Vec<ThreadLine>,
    burst: &'a Report,
}

#[derive(Serialize)]
struct ThreadLine {
    name: String,
    tid: u64,
    cpu_ms: u64,
}

fn build_line<'a>(id: usize, entry: &Entry, burst: &'a Report) -> Line<'a> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let windows = transport_windows(crate::transfer::memory_budget());
    let busiest = busiest_threads(&entry.threads_at_start, &read_thread_cpu(), BUSIEST_THREADS);
    Line {
        kind: "share_quic_burst",
        ts_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0)),
        pid: std::process::id(),
        side: entry.side.name(),
        conn: id,
        seq: SEQUENCE.fetch_add(1, Ordering::Relaxed),
        cc: controller_name(),
        windows: Windows {
            stream_bytes: windows.stream,
            connection_bytes: windows.connection,
            send_bytes: windows.send,
        },
        socket_buffers: read_buffer_limits(),
        busiest_threads: busiest
            .into_iter()
            .map(|thread| ThreadLine {
                name: thread.name,
                tid: thread.tid,
                cpu_ms: thread.cpu_ms,
            })
            .collect(),
        burst,
    }
}

fn write_burst(id: usize, entry: &Entry, burst: &Report) {
    let Ok(text) = serde_json::to_string(&build_line(id, entry, burst)) else {
        return;
    };
    // A diagnosis that cannot be written is dropped: it must never disturb a
    // transfer.
    let _ = append_line(&crate::support_dirs::app_data_file(STATS_FILE), &text);
}

/// Whether the file must make room before `incoming` more bytes.
pub(super) fn needs_rotation(length: u64, incoming: usize) -> bool {
    length.saturating_add(incoming as u64) > MAX_FILE_BYTES
}

/// Where the replaced file goes: `<name>.1`.
pub(super) fn rotated_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

fn append_line(path: &Path, line: &str) -> io::Result<()> {
    if let Ok(metadata) = std::fs::metadata(path) {
        if needs_rotation(metadata.len(), line.len() + 1) {
            let _ = std::fs::rename(path, rotated_path(path));
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    file.write_all(&bytes)
}
