//! Platform-neutral types of the change-watch service (RV1, contract V4).

use std::fmt;
use std::sync::Arc;

/// Identity of one root watch, unique for the lifetime of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WatchId(u64);

impl WatchId {
    pub(crate) const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// What happened to a path below the root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    Created,
    Removed,
    /// Content changed (size or write time).
    Modified,
    /// Only attributes, permissions or times changed.
    Metadata,
    /// Old name of a rename or of a move out of the root.
    RenamedFrom,
    /// New name of a rename or of a move into the root.
    RenamedTo,
    /// Something at or below `rel` changed; details unknown (host signal).
    Unknown,
}

/// One change below the root.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Change {
    /// Path relative to the root with `/` separators; empty = the root itself
    /// (only for `EventKind::Unknown`). Names that are not valid UTF-8 are
    /// converted lossily: events are triggers, the rescan reads the real name.
    pub rel: String,
    pub kind: EventKind,
    /// `Some(true)` for a directory; `None` when the OS does not say (Windows).
    pub is_dir: Option<bool>,
}

/// How completely an armed watch reports changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Coverage {
    /// Every change of the watched local file system is reported.
    Complete,
    /// Only changes made through this machine are reported (Linux NFS,
    /// SMB/CIFS, FUSE and other network mounts), or completeness is unknown:
    /// poll as well, including Windows SMB redirectors.
    LocalOnly,
    /// Android shared storage: writes the media provider makes on the lower
    /// file system are not reported; host signals (`report_host_change`) and
    /// verification runs cover them.
    SharedStorage,
}

/// Why a root is not, or no longer, watched.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UnavailableReason {
    /// The inotify watch or instance limit is reached (`ENOSPC`, `EMFILE`);
    /// the service arms the root again with backoff.
    WatchLimit,
    /// The file system or network redirector cannot report changes; final for
    /// this watch.
    Unsupported,
    /// The root does not exist, was removed, renamed or unmounted; armed again
    /// when it is back.
    RootMissing,
    /// Windows: the volume is being removed and its handle was closed so the
    /// removal can succeed; armed again when the volume arrives.
    DeviceRemoved,
    /// The root cannot be read; armed again with backoff.
    AccessDenied,
    /// Other operating-system error (text for the log); armed again with
    /// backoff.
    Failed(String),
}

/// One message of a watch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    /// The whole tree is armed. The first `Ready` follows `watch`; changes
    /// before it are not all reported (the consumer verifies once). A later
    /// `Ready` follows `Unavailable`: changes in between are unknown (rescan).
    Ready(Coverage),
    Change(Change),
    /// Events were lost (kernel queue, change buffer, full sink): rescan the
    /// whole root.
    Overflow,
    /// Watching stopped; poll until the next `Ready`.
    Unavailable(UnavailableReason),
}

/// A message together with the watch it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchMessage {
    pub id: WatchId,
    pub event: WatchEvent,
}

/// Settings of one watch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WatchOptions {
    /// Also watch other file systems mounted below the root (Linux, Android).
    /// Off by default, like `SyncJob::cross_mounts` for new jobs.
    pub cross_mounts: bool,
}

/// An entry the consumer filter decides on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchEntry<'a> {
    /// Path relative to the root with `/` separators (never empty).
    pub rel: &'a str,
    /// `Some(true)` for a directory; `None` when unknown.
    pub is_dir: Option<bool>,
}

type FilterFn = dyn Fn(&WatchEntry<'_>) -> bool + Send + Sync;

/// Decides which entries matter to the consumer. `false` drops the event;
/// for a directory it also excludes everything below it (not watched on
/// Linux and Android, its events dropped on Windows). It runs on the service
/// thread: no blocking, no I/O.
#[derive(Clone)]
pub struct WatchFilter(Arc<FilterFn>);

impl WatchFilter {
    pub fn new(admit: impl Fn(&WatchEntry<'_>) -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(admit))
    }

    /// Admits every entry.
    pub fn all() -> Self {
        Self::new(|_| true)
    }

    pub fn admits(&self, entry: &WatchEntry<'_>) -> bool {
        (self.0)(entry)
    }
}

impl Default for WatchFilter {
    fn default() -> Self {
        Self::all()
    }
}

impl fmt::Debug for WatchFilter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WatchFilter(..)")
    }
}

/// Where the messages of a watch go.
#[derive(Clone)]
pub enum WatchSink {
    /// Delivered with `try_send`: messages that do not fit are dropped and
    /// reported by one later `Overflow`; a disconnected receiver ends the
    /// watch.
    Channel(crossbeam_channel::Sender<WatchMessage>),
    /// Called on the service thread: return quickly, never create or drop a
    /// watch from it.
    Callback(Arc<dyn Fn(&WatchMessage) + Send + Sync>),
}

/// Result of handing one message to a sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    Delivered,
    /// The channel is full; the message was dropped.
    Full,
    /// The receiver is gone; the watch ends.
    Gone,
}

impl WatchSink {
    pub fn callback(deliver: impl Fn(&WatchMessage) + Send + Sync + 'static) -> Self {
        Self::Callback(Arc::new(deliver))
    }

    pub(crate) fn deliver(&self, message: WatchMessage) -> Delivery {
        match self {
            Self::Channel(sender) => match sender.try_send(message) {
                Ok(()) => Delivery::Delivered,
                Err(crossbeam_channel::TrySendError::Full(_)) => Delivery::Full,
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => Delivery::Gone,
            },
            Self::Callback(deliver) => {
                deliver(&message);
                Delivery::Delivered
            }
        }
    }
}

impl From<crossbeam_channel::Sender<WatchMessage>> for WatchSink {
    fn from(sender: crossbeam_channel::Sender<WatchMessage>) -> Self {
        Self::Channel(sender)
    }
}

impl fmt::Debug for WatchSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Channel(_) => formatter.write_str("WatchSink::Channel(..)"),
            Self::Callback(_) => formatter.write_str("WatchSink::Callback(..)"),
        }
    }
}
