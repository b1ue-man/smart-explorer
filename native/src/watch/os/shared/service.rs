//! The process-wide watch service (RV1, contract V4): the registry of watched
//! roots, delivery to the consumers' sinks (filter, own directories,
//! coalesced `Overflow` for full channels) and the lazily started platform
//! backend (Linux/Android: one inotify instance for all roots; Windows:
//! `ReadDirectoryChangesW` per root with handle release on volume removal).

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::backend::Backend;
use super::paths::{ancestors, is_own};
use super::types::{
    Delivery, UnavailableReason, WatchEntry, WatchEvent, WatchFilter, WatchId, WatchMessage,
    WatchOptions, WatchSink,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static SERVICE: Mutex<Service> = Mutex::new(Service {
    watches: BTreeMap::new(),
    backend: None,
});
/// Directory verdicts remembered per watch before the cache starts over.
const DIR_CACHE_LIMIT: usize = 4_096;

/// What the backend needs to arm one root.
#[derive(Clone)]
pub(crate) struct RootSpec {
    pub(crate) id: WatchId,
    /// Canonical job root, or a literal display path for a confined watch.
    pub(crate) root: PathBuf,
    /// An already authorized root. Never reopen `root` when this is present.
    pub(crate) anchor: Option<crate::local_access::DirectoryHandle>,
    /// Used where directories are watched one by one (inotify); Windows
    /// watches whole subtrees and filters in `emit`.
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) options: WatchOptions,
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) filter: WatchFilter,
}

struct Registered {
    root: PathBuf,
    anchor: Option<crate::local_access::DirectoryHandle>,
    filter: WatchFilter,
    sink: WatchSink,
    /// A message did not fit; one `Overflow` is still owed.
    overflow_owed: bool,
    dir_verdicts: HashMap<String, bool>,
}

struct Service {
    watches: BTreeMap<WatchId, Registered>,
    backend: Option<Backend>,
}

fn service() -> MutexGuard<'static, Service> {
    // Every update leaves the registry consistent; a panic elsewhere must not
    // take watching down with it.
    SERVICE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The app's own data and cache directories, never watched or reported. The
/// desktop has no cache directory of its own (it uses the shared system temp
/// directory, which stays watchable).
pub(crate) fn own_directories() -> Vec<PathBuf> {
    let mut own = vec![crate::support_dirs::app_data_dir()];
    if let Some(host) = crate::support_dirs::host() {
        own.push(host.cache_dir.clone());
    }
    own.into_iter()
        .map(|dir| std::fs::canonicalize(&dir).unwrap_or(dir))
        .filter(|dir| dir.parent().is_some())
        .collect()
}

/// One watched root. Dropping it stops the watch; messages already handed to
/// the sink may still arrive afterwards, so consumers ignore unknown ids.
#[derive(Debug)]
pub struct WatchHandle {
    id: WatchId,
    root: PathBuf,
}

impl WatchHandle {
    pub fn id(&self) -> WatchId {
        self.id
    }

    /// The root as passed to `watch`.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        let mut service = service();
        service.watches.remove(&self.id);
        if let Some(backend) = &service.backend {
            backend.remove(self.id);
        }
    }
}

/// Starts watching the tree below `root` (an absolute path). Returns at once:
/// the watch is armed on the service thread and reports `Ready(coverage)` or
/// `Unavailable(reason)` through `sink`, then changes. The app data and cache
/// directories are never watched or reported; links and junctions below the
/// root are never followed. Errors: `InvalidInput` for a relative root.
pub fn watch(
    root: &Path,
    options: WatchOptions,
    filter: WatchFilter,
    sink: WatchSink,
) -> io::Result<WatchHandle> {
    if !root.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "watch root must be an absolute path",
        ));
    }
    // A root that does not exist yet keeps its literal path; the backend
    // reports it missing and arms it once it appears.
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    register(root, canonical, None, options, filter, sink)
}

/// Watches an already authorized directory without resolving its display
/// path again. Linux/Android watch the held object and its immediate entries;
/// `Ready(LocalOnly)` requires polling for deeper changes. Windows cannot
/// derive an overlapped watch from its read pin and returns `Unsupported`.
/// A missing root must be opened safely by the caller before using this API.
pub(crate) fn watch_confined(
    anchor: &crate::local_access::DirectoryHandle,
    literal_root: &Path,
    options: WatchOptions,
    filter: WatchFilter,
    sink: WatchSink,
) -> io::Result<WatchHandle> {
    if !literal_root.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "watch root must be absolute"));
    }
    if anchor.watch_path().is_none() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "directory pin has no safe watch capability"));
    }
    register(literal_root, literal_root.to_path_buf(), Some(anchor.clone()), options, filter, sink)
}

fn register(
    display_root: &Path,
    resolved_root: PathBuf,
    anchor: Option<crate::local_access::DirectoryHandle>,
    options: WatchOptions,
    filter: WatchFilter,
    sink: WatchSink,
) -> io::Result<WatchHandle> {
    let id = WatchId::new(NEXT_ID.fetch_add(1, Ordering::Relaxed));
    let spec = RootSpec {
        id,
        root: resolved_root.clone(),
        anchor: anchor.clone(),
        options,
        filter: filter.clone(),
    };
    let mut service = service();
    service.watches.insert(
        id,
        Registered {
            root: resolved_root,
            anchor,
            filter,
            sink,
            overflow_owed: false,
            dir_verdicts: HashMap::new(),
        },
    );
    if service.backend.is_none() {
        match Backend::start() {
            Ok(backend) => service.backend = Some(backend),
            Err(error) => {
                drop(service);
                emit(
                    id,
                    vec![WatchEvent::Unavailable(UnavailableReason::Failed(format!(
                        "Überwachung nicht startbar: {error}"
                    )))],
                );
                return Ok(WatchHandle {
                    id,
                    root: display_root.to_path_buf(),
                });
            }
        }
    }
    if let Some(backend) = &service.backend {
        backend.add(spec);
    }
    Ok(WatchHandle {
        id,
        root: display_root.to_path_buf(),
    })
}

/// Whether a consumer filter admits a change (own directories excluded,
/// every ancestor directory admitted, then the entry itself).
fn admits(watch: &mut Registered, own: &[PathBuf], event: &WatchEvent) -> bool {
    let WatchEvent::Change(change) = event else {
        return true;
    };
    if change.rel.is_empty() {
        return true;
    }
    if is_own(&watch.root.join(&change.rel), own) {
        return false;
    }
    for dir in ancestors(&change.rel) {
        let verdict = match watch.dir_verdicts.get(dir) {
            Some(verdict) => *verdict,
            None => {
                let verdict = watch.filter.admits(&WatchEntry {
                    rel: dir,
                    is_dir: Some(true),
                });
                if watch.dir_verdicts.len() >= DIR_CACHE_LIMIT {
                    watch.dir_verdicts.clear();
                }
                watch.dir_verdicts.insert(dir.to_string(), verdict);
                verdict
            }
        };
        if !verdict {
            return false;
        }
    }
    watch.filter.admits(&WatchEntry {
        rel: &change.rel,
        is_dir: change.is_dir,
    })
}

/// Hands events of one watch to its sink (called by the backends and the host
/// signals). Filtered changes are dropped; a full channel turns the rest into
/// one owed `Overflow`; a gone receiver ends the watch.
pub(crate) fn emit(id: WatchId, events: Vec<WatchEvent>) {
    if events.is_empty() {
        return;
    }
    let own = own_dirs_cached();
    let (sink, events, owed, _anchor) = {
        let mut service = service();
        let Some(watch) = service.watches.get_mut(&id) else {
            return;
        };
        let mut admitted: Vec<WatchEvent> = Vec::with_capacity(events.len());
        for event in events {
            if admits(watch, &own, &event) && admitted.last() != Some(&event) {
                admitted.push(event);
            }
        }
        if admitted.is_empty() && !watch.overflow_owed {
            return;
        }
        (
            watch.sink.clone(),
            admitted,
            std::mem::take(&mut watch.overflow_owed),
            // Keep the authorized object alive even if the sink drops the
            // WatchHandle while delivery runs outside the registry lock.
            watch.anchor.clone(),
        )
    };
    let owed_overflow = owed.then_some(WatchEvent::Overflow);
    for event in owed_overflow.into_iter().chain(events) {
        match sink.deliver(WatchMessage { id, event }) {
            Delivery::Delivered => {}
            Delivery::Full => {
                if let Some(watch) = service().watches.get_mut(&id) {
                    watch.overflow_owed = true;
                }
                return;
            }
            Delivery::Gone => {
                let mut service = service();
                service.watches.remove(&id);
                if let Some(backend) = &service.backend {
                    backend.remove(id);
                }
                return;
            }
        }
    }
}

/// Retries owed `Overflow` messages; backends call it about every second
/// while `overflow_owed()` says so.
pub(crate) fn deliver_owed() {
    let owed: Vec<WatchId> = service()
        .watches
        .iter()
        .filter(|(_, watch)| watch.overflow_owed)
        .map(|(id, _)| *id)
        .collect();
    for id in owed {
        emit(id, vec![WatchEvent::Overflow]);
    }
}

pub(crate) fn overflow_owed() -> bool {
    service().watches.values().any(|watch| watch.overflow_owed)
}

/// Path-authorized roots for host signals. A host path alone cannot identify
/// the opened object of a confined watch after an ancestor/root swap.
pub(crate) fn roots() -> Vec<(WatchId, PathBuf)> {
    service()
        .watches
        .iter()
        .filter(|(_, watch)| watch.anchor.is_none())
        .map(|(id, watch)| (*id, watch.root.clone()))
        .collect()
}

fn own_dirs_cached() -> Vec<PathBuf> {
    static OWN: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    OWN.get_or_init(own_directories).clone()
}
