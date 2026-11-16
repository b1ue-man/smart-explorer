//! Metadata presence (plan 2026-11-14 E3, decision of 2026-11-14 on E4): a
//! listing, a status query or another small file-system control operation
//! that a person started is interactive use of the device it asks. While one
//! runs to a device, and for a hold time after the last one
//! (`yield_engagement.rs`), bulk file connections to that device yield
//! (`yield_controller.rs`), as they do for a screen session.
//!
//! Unlike the sessions of `interactive_presence.rs` (process-wide), this
//! presence is per remote device: browsing one device leaves transfers to
//! every other device alone. Both sides set it: the client for the duration
//! of its request (its uploads to that device yield), the host for the
//! duration of its answer to a request the client marked as started by a
//! person (its sends to that device yield). The job engine (copy, sync,
//! compare, hash walks, storage analysis) never sets it: it issues such
//! operations by the thousand, and its own transfers would brake themselves.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use iroh::EndpointId;
use serde::{Deserialize, Serialize};

/// Who started a file-system operation on a Share device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationOrigin {
    /// A person waits for the answer: the Explorer view, `se ls`, `se stat`
    /// and the other single commands, the MCP file tools, a drive.
    User,
    /// The job engine and everything not known to be a person: no presence.
    #[default]
    Job,
}

/// The activity of one remote device.
#[derive(Debug)]
pub(crate) struct PeerActivity {
    epoch: Instant,
    running: AtomicUsize,
    /// End of the last operation in nanoseconds after `epoch`, plus one; 0
    /// before the first one ended.
    last_end: AtomicU64,
}

impl PeerActivity {
    fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            running: AtomicUsize::new(0),
            last_end: AtomicU64::new(0),
        }
    }

    /// Whether an operation to this device runs now.
    pub(crate) fn running(&self) -> bool {
        self.running.load(Ordering::Acquire) > 0
    }

    /// When the last operation to this device ended.
    pub(crate) fn last_end(&self) -> Option<Instant> {
        match self.last_end.load(Ordering::Acquire) {
            0 => None,
            stamp => Some(self.epoch + Duration::from_nanos(stamp - 1)),
        }
    }

    fn begin(&self) {
        self.running.fetch_add(1, Ordering::AcqRel);
    }

    fn end(&self, now: Instant) {
        let nanos = now.saturating_duration_since(self.epoch).as_nanos();
        let stamp = u64::try_from(nanos)
            .unwrap_or(u64::MAX - 1)
            .saturating_add(1);
        self.last_end.fetch_max(stamp, Ordering::AcqRel);
        // Never below zero: every guard counted once in `begin`.
        let _ = self
            .running
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_sub(1)
            });
    }
}

/// The activity of every remote device this process talks to.
#[derive(Debug)]
pub(crate) struct MetadataPresence {
    epoch: Instant,
    peers: Mutex<HashMap<EndpointId, Arc<PeerActivity>>>,
}

impl MetadataPresence {
    /// An empty register (tests; the process uses [`global`]).
    pub(crate) fn new() -> Self {
        Self {
            epoch: Instant::now(),
            peers: Mutex::new(HashMap::new()),
        }
    }

    /// The activity of `peer`; a connection's controller binds it once
    /// (`PurposeSlot::bind_peer`). Entries nobody holds and that have ended
    /// go when another is added, so the map stays at the live devices.
    pub(crate) fn activity(&self, peer: EndpointId) -> Arc<PeerActivity> {
        let mut peers = self.peers.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = peers.get(&peer) {
            return Arc::clone(found);
        }
        peers.retain(|_, activity| Arc::strong_count(activity) > 1 || activity.running());
        let activity = Arc::new(PeerActivity::new(self.epoch));
        peers.insert(peer, Arc::clone(&activity));
        activity
    }

    /// Registers one operation of `origin` to `peer` until the guard drops;
    /// `None` for the job engine.
    pub(crate) fn begin(&self, peer: EndpointId, origin: OperationOrigin) -> Option<MetadataGuard> {
        if origin != OperationOrigin::User {
            return None;
        }
        let activity = self.activity(peer);
        activity.begin();
        Some(MetadataGuard { activity })
    }
}

/// The origin of an operation that a handle opened for `opened` makes on
/// the calling thread: work the program started by itself
/// (`crate::vfs::BackgroundScope`: a cache refresh, a preload, an upload)
/// is `Job` whatever the handle was opened for, because nobody waits for
/// its answer (E4c).
pub(crate) fn in_this_thread(opened: OperationOrigin) -> OperationOrigin {
    if crate::vfs::in_background() {
        OperationOrigin::Job
    } else {
        opened
    }
}

/// The register of this process.
pub(crate) fn global() -> &'static MetadataPresence {
    static PRESENCE: OnceLock<MetadataPresence> = OnceLock::new();
    PRESENCE.get_or_init(MetadataPresence::new)
}

/// Registers one operation of `origin` to `peer` in the process register.
/// The client calls it before it sends a request, the host when it starts
/// to answer a request marked `user` (`framing::recv_ctrl_with_origin`).
pub(crate) fn begin(peer: EndpointId, origin: OperationOrigin) -> Option<MetadataGuard> {
    global().begin(peer, origin)
}

/// [`begin`] for a device known by its node id text (`PeerPresence::node_id`);
/// `None` when the text is no node id.
pub(crate) fn begin_for_node(node_id: &str, origin: OperationOrigin) -> Option<MetadataGuard> {
    if origin != OperationOrigin::User {
        return None;
    }
    begin(node_id.parse().ok()?, origin)
}

/// One running operation; dropping it ends the operation (also on error,
/// cancellation, panic unwinding and task abort).
#[must_use = "the operation counts as interactive only while the guard lives"]
#[derive(Debug)]
pub(crate) struct MetadataGuard {
    activity: Arc<PeerActivity>,
}

impl Drop for MetadataGuard {
    fn drop(&mut self) {
        self.activity.end(Instant::now());
    }
}

/// The JSON of `Ctrl::Fs` with its origin: a `user` request carries the
/// field `"origin":"user"`; hosts before this field ignore it (serde ignores
/// unknown fields of `Ctrl`), and a request without it counts as `job`.
pub(crate) fn fs_ctrl_payload(
    ctrl: &crate::share::wire::Ctrl,
    origin: OperationOrigin,
) -> serde_json::Result<Vec<u8>> {
    if origin != OperationOrigin::User {
        return serde_json::to_vec(ctrl);
    }
    let mut value = serde_json::to_value(ctrl)?;
    if let Some(fields) = value.as_object_mut() {
        fields.insert("origin".to_owned(), serde_json::to_value(origin)?);
    }
    serde_json::to_vec(&value)
}

/// The origin a control frame states; `job` without one or with a value
/// this version does not know.
pub(crate) fn origin_of(payload: &[u8]) -> OperationOrigin {
    #[derive(Deserialize)]
    struct Stated {
        #[serde(default)]
        origin: OperationOrigin,
    }
    serde_json::from_slice::<Stated>(payload)
        .map(|stated| stated.origin)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "metadata_presence_tests.rs"]
mod tests;
