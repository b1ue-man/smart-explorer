use std::sync::Mutex;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};

use super::types::ShareEvent;

pub(super) const SHARE_EVENT_CAPACITY: usize = 512;
const REPORT_INTERVAL: Duration = Duration::from_secs(30);
const MAX_DETAIL_BYTES: usize = 512;

// Keep this a fixed enum/array. Remote-controlled error text must never become
// a map key because rotating details would turn diagnostics into a memory sink.
#[derive(Clone, Copy, Debug)]
pub(super) enum ConnectionErrorKind {
    Accept,
    FsConnection,
    ExecConnection,
    FsStream,
}

impl ConnectionErrorKind {
    const COUNT: usize = 4;

    fn index(self) -> usize {
        match self {
            Self::Accept => 0,
            Self::FsConnection => 1,
            Self::ExecConnection => 2,
            Self::FsStream => 3,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Accept => "Iroh-Accept",
            Self::FsConnection => "Iroh-FS-Verbindung",
            Self::ExecConnection => "Iroh-Exec-Verbindung",
            Self::FsStream => "Iroh-FS",
        }
    }
}

#[derive(Default)]
struct ErrorBucket {
    last_sent: Option<Instant>,
    pending: u64,
    latest_detail: String,
}

pub(super) struct ConnectionEventReporter {
    buckets: Mutex<[ErrorBucket; ConnectionErrorKind::COUNT]>,
}

impl Default for ConnectionEventReporter {
    fn default() -> Self {
        Self {
            buckets: Mutex::new(std::array::from_fn(|_| ErrorBucket::default())),
        }
    }
}

impl ConnectionEventReporter {
    pub(super) fn report(
        &self,
        kind: ConnectionErrorKind,
        detail: impl AsRef<str>,
        events: &Sender<ShareEvent>,
    ) {
        self.report_at(kind, detail.as_ref(), events, Instant::now());
    }

    fn report_at(
        &self,
        kind: ConnectionErrorKind,
        detail: &str,
        events: &Sender<ShareEvent>,
        now: Instant,
    ) {
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let bucket = &mut buckets[kind.index()];
        bucket.pending = bucket.pending.saturating_add(1);
        bucket.latest_detail = truncate_detail(detail);

        let due = bucket
            .last_sent
            .map(|sent| now.saturating_duration_since(sent) >= REPORT_INTERVAL)
            .unwrap_or(true);
        if !due {
            return;
        }

        let suppressed = bucket.pending.saturating_sub(1);
        let message = if suppressed == 0 {
            format!("{}: {}", kind.label(), bucket.latest_detail)
        } else {
            format!(
                "{}: {} ({suppressed} weitere gleichartige Fehler unterdrueckt)",
                kind.label(),
                bucket.latest_detail
            )
        };
        // An unauthenticated peer must not block an accept/handshake task when
        // the UI or daemon has not drained events yet. Preserve the coalesced
        // count and latest bounded detail so a later attempt can report them.
        if events.try_send(ShareEvent::Error(message)).is_ok() {
            bucket.last_sent = Some(now);
            bucket.pending = 0;
        }
    }
}

pub(super) fn channel() -> (Sender<ShareEvent>, Receiver<ShareEvent>) {
    // This matches the daemon's retained UI-event ceiling. Unlike an unbounded
    // linked-list channel, producer bursts now have a hard memory boundary.
    bounded(SHARE_EVENT_CAPACITY)
}

fn truncate_detail(detail: &str) -> String {
    if detail.len() <= MAX_DETAIL_BYTES {
        return detail.to_string();
    }
    const MARKER: &str = "...";
    let mut end = MAX_DETAIL_BYTES - MARKER.len();
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = detail[..end].to_string();
    truncated.push_str(MARKER);
    truncated
}

#[cfg(test)]
#[path = "connection_events_tests.rs"]
mod tests;
