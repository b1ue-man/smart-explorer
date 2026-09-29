//! Credit-based flow control per request (`credit-v1`).
//!
//! Every request of a credit connection starts with `CREDIT_INITIAL` bytes
//! of implicit credit in each direction. The receiver of a stream returns
//! credit with `Frame::Credit` as its consumer takes frames, so no sender
//! ever has more stream bytes queued at the receiver than it was granted.
//! Frames without credit are small by protocol (texts of batch item frames
//! are limited, see `batch_limits`) and few: an upload may carry one trailer
//! per batch entry and its `End`; a server sends one reply per request, one
//! or two item frames per batch item and progress at most every 200 ms.
//! Receivers therefore queue without bound in frames but bounded in bytes,
//! and a reader thread never blocks on one slow consumer (head-of-line).
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::types::Frame;

/// File data one connection may hold queued at a receiver for all streams:
/// today's worst case of an agent connection (8 requests x 32 frames x
/// 256 KiB), so the higher request limit does not raise the memory ceiling.
pub const CREDIT_CONNECTION_BUDGET: u64 = 64 * 1024 * 1024;
/// Implicit credit of every request and direction. Budget / request limit,
/// so the implicit credit of a full connection stays inside the budget; it
/// covers four full data frames before the first grant returns.
pub const CREDIT_INITIAL: u64 = 1024 * 1024;
/// Largest per-stream window: today's per-request pipelining depth
/// (`TRANSFER_FRAME_BACKLOG` x `CHUNK`), so one stream keeps reaching
/// 8 MiB per round trip (about 670 Mbit/s at 100 ms) as before.
pub const CREDIT_WINDOW_MAX: u64 = 8 * 1024 * 1024;
/// Concurrent requests a credit connection admits: budget / initial credit.
pub const CREDIT_REQUEST_LIMIT: usize = (CREDIT_CONNECTION_BUDGET / CREDIT_INITIAL) as usize;
/// Highest cost of one frame. Half the smallest window, so a receiver whose
/// outstanding credit is too small for the next frame always grants again
/// (a grant is due once outstanding credit falls to half the window).
const FRAME_COST_MAX: u64 = CREDIT_INITIAL / 2;

/// Wire tags of the charged frames: `Data`, `TreeEntry`, `Match`, `HashEntry`.
const CHARGED_TAGS: [u8; 4] = [11, 18, 20, 22];

/// Frames whose number a stream does not bound consume credit. Replies,
/// terminals, control frames and the per-item batch frames stay free: a
/// request has one reply, a batch at most `BATCH_MAX_FILES` items, and a
/// free outcome frame can never wait for credit behind the peer's upload.
pub fn charged(frame: &Frame) -> bool {
    matches!(
        frame,
        Frame::Data(_) | Frame::TreeEntry { .. } | Frame::Match { .. } | Frame::HashEntry { .. }
    )
}

/// Credit a frame consumes: its encoded body length, capped.
pub fn credit_cost(frame: &Frame) -> u64 {
    if !charged(frame) {
        return 0;
    }
    let length = frame
        .wire_len()
        .map_or(FRAME_COST_MAX, |length| length as u64);
    length.min(FRAME_COST_MAX)
}

/// Credit of one frame as `write_frame` emits it (length prefix, request id,
/// tag, fields). Anything that is not exactly one frame costs nothing.
pub(crate) fn encoded_cost(bytes: &[u8]) -> u64 {
    if bytes.len() < 13 {
        return 0;
    }
    let mut prefix = [0u8; 4];
    prefix.copy_from_slice(&bytes[..4]);
    let body = u32::from_le_bytes(prefix) as usize;
    if body.checked_add(4) != Some(bytes.len()) || !CHARGED_TAGS.contains(&bytes[12]) {
        return 0;
    }
    (body as u64).min(FRAME_COST_MAX)
}

/// Window a stream gets while `streams` streams of its connection receive.
pub fn window_target(streams: usize) -> u64 {
    (CREDIT_CONNECTION_BUDGET / streams.max(1) as u64).clamp(CREDIT_INITIAL, CREDIT_WINDOW_MAX)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Credit the peer granted for the stream frames we send on one request.
pub struct SendCredit {
    state: Mutex<SendState>,
    changed: Condvar,
}

struct SendState {
    available: u64,
    closed: bool,
}

impl Default for SendCredit {
    fn default() -> Self {
        Self::new()
    }
}

impl SendCredit {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(SendState {
                available: CREDIT_INITIAL,
                closed: false,
            }),
            changed: Condvar::new(),
        }
    }

    pub fn grant(&self, bytes: u64) {
        let mut state = lock(&self.state);
        state.available = state.available.saturating_add(bytes);
        self.changed.notify_all();
    }

    /// The request ended or was canceled: every waiter fails.
    pub fn close(&self) {
        lock(&self.state).closed = true;
        self.changed.notify_all();
    }

    /// Wait until `cost` bytes may be sent and take them. `stall` fails the
    /// wait when that long passes without any new grant.
    pub fn take(&self, cost: u64, stall: Option<Duration>) -> io::Result<()> {
        let mut state = lock(&self.state);
        let mut seen = state.available;
        let mut deadline = stall.map(|stall| Instant::now() + stall);
        loop {
            if state.closed {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Anfrage beendet, bevor die Gegenstelle weitere Daten annahm",
                ));
            }
            if state.available >= cost {
                state.available -= cost;
                return Ok(());
            }
            state = match deadline {
                None => self
                    .changed
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Gegenstelle nimmt keine weiteren Daten an",
                        ));
                    }
                    self.changed
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .0
                }
            };
            if state.available > seen {
                seen = state.available;
                deadline = stall.map(|stall| Instant::now() + stall);
            }
        }
    }
}

/// Streams of one connection that currently receive data; they share the
/// connection budget.
#[derive(Default)]
pub struct StreamCount(AtomicUsize);

impl StreamCount {
    pub fn current(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// Receive-side accounting of one request: what we granted, what arrived
/// and what our consumer took.
pub struct RecvWindow {
    granted: AtomicU64,
    received: AtomicU64,
    consumed: AtomicU64,
    streaming: AtomicBool,
    streams: std::sync::Arc<StreamCount>,
}

impl RecvWindow {
    pub fn new(streams: std::sync::Arc<StreamCount>) -> Self {
        Self {
            granted: AtomicU64::new(CREDIT_INITIAL),
            received: AtomicU64::new(0),
            consumed: AtomicU64::new(0),
            streaming: AtomicBool::new(false),
            streams,
        }
    }

    /// Reader side: account an arrived frame. False = the peer sent more
    /// than it was granted (a protocol violation).
    pub fn receive(&self, cost: u64) -> bool {
        if cost == 0 {
            return true;
        }
        let received = self
            .received
            .fetch_add(cost, Ordering::SeqCst)
            .saturating_add(cost);
        received <= self.granted.load(Ordering::SeqCst)
    }

    /// Consumer side: account a taken frame. Returns the credit to grant
    /// now; a grant is due once outstanding credit fell to half the window.
    pub fn consume(&self, cost: u64) -> Option<u64> {
        if cost == 0 {
            return None;
        }
        if !self.streaming.swap(true, Ordering::SeqCst) {
            self.streams.0.fetch_add(1, Ordering::SeqCst);
        }
        let consumed = self
            .consumed
            .fetch_add(cost, Ordering::SeqCst)
            .saturating_add(cost);
        let target = window_target(self.streams.current());
        let outstanding = self.granted.load(Ordering::SeqCst).saturating_sub(consumed);
        let deficit = target.saturating_sub(outstanding);
        if deficit == 0 || deficit < target / 2 {
            return None;
        }
        self.granted.fetch_add(deficit, Ordering::SeqCst);
        Some(deficit)
    }
}

impl Drop for RecvWindow {
    fn drop(&mut self) {
        if self.streaming.load(Ordering::SeqCst) {
            self.streams.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Marker of a congestion reply inside an ordinary error frame; only sent
/// to credit connections, older clients keep the plain text.
pub const BUSY_MARKER: &str = "SE_BUSY_V1";

/// Rejection texts of servers without `credit-v1`.
pub const LEGACY_BUSY_TEXTS: [&str; 2] = [
    "too many concurrent agent requests",
    "too many concurrent backend requests",
];

/// Error text that tells a credit client to slow down (`retry_after` when
/// the peer named a delay).
pub fn busy_message(retry_after: Option<Duration>, message: &str) -> String {
    let millis = retry_after.map_or(0, |delay| delay.as_millis().min(u64::MAX as u128) as u64);
    format!("{BUSY_MARKER} retry_ms={millis}: {message}")
}

/// The delay and message of a congestion reply, if `text` is one.
pub fn parse_busy(text: &str) -> Option<(Option<Duration>, &str)> {
    if let Some(rest) = text.strip_prefix(BUSY_MARKER) {
        let (millis, message) = rest.strip_prefix(" retry_ms=")?.split_once(": ")?;
        let millis: u64 = millis.parse().ok()?;
        return Some((
            (millis > 0).then_some(Duration::from_millis(millis)),
            message,
        ));
    }
    LEGACY_BUSY_TEXTS.contains(&text).then_some((None, text))
}
