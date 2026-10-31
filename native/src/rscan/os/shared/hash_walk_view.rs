//! One request for a whole remote tree: `vfs::hash_walk` with metadata replaces
//! one listing per folder. The rows are the ones the folder walk gives
//! (`FileEntry` fields, depth, budget, protected omissions, the folder depth
//! limit); only their arrival differs. It is used without retention and
//! without a depth limit. A filtered view keeps the folder walk: placing a
//! retained entry needs the chain of every pruned ancestor folder, and this
//! stream would have to keep that chain for the whole tree, while the scan
//! budget counts only emitted entries.
use super::budget::MAX_SCAN_DEPTH;
use super::walk_state::WalkState;
use super::{ext_of, join};
use crate::types::FileEntry;
use crate::vfs::{BackendHandle, HashWalkEntry, HashWalkItem, HashWalkRequest, VfsOmission};
use crossbeam_channel::{bounded, RecvTimeoutError};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Items queued between the walk and the driver; the walk waits while the
/// driver is behind.
const QUEUE_ITEMS: usize = 1024;
/// How often a quiet walk is checked for a stop.
const STOP_POLL: Duration = Duration::from_millis(100);

/// Walks the whole tree of `root` with one request and feeds its items to
/// `state`. Returns `false` when no row was taken: the storing side cannot walk
/// this way, or it failed before its first row, and the caller lists folder by
/// folder. Returns `true` when the walk was taken: complete, stopped, or failed
/// after rows (the failure is then the scan's error, and the rows stay).
pub(super) fn walk_in_one_request(
    backend: &BackendHandle,
    root: &str,
    state: &mut WalkState,
) -> bool {
    // Storing sides without the capability (most backends) are walked folder by
    // folder at once, without a thread and without a channel.
    if !matches!(crate::vfs::supports_hash_walk(&**backend, root), Ok(true)) {
        return false;
    }
    let (items_tx, items_rx) = bounded::<HashWalkItem>(QUEUE_ITEMS);
    let cancel = state.cancel_handle();
    let request = HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta: true,
    };
    let mut rows = Rows::new(root);
    let outcome = std::thread::scope(|scope| {
        let walk_cancel = cancel.clone();
        let walker = std::thread::Builder::new()
            .name("rscan-walk".into())
            .spawn_scoped(scope, move || {
                // The stream lasts until the call returns, so the driver ends
                // with the walk itself and not with an earlier sender drop.
                let stream_lifetime = items_tx.clone();
                let outcome =
                    crate::vfs::hash_walk(&**backend, root, request, items_tx, &walk_cancel);
                drop(stream_lifetime);
                outcome
            })
            .ok()?;
        let mut stop_early = false;
        loop {
            match items_rx.recv_timeout(STOP_POLL) {
                Ok(item) => {
                    if !rows.take(state, item) {
                        stop_early = true;
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if state.stopped() {
                        stop_early = true;
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if stop_early {
            cancel.store(true, Ordering::Relaxed);
        }
        drop(items_rx);
        Some(walker.join())
    });
    let Some(outcome) = outcome else {
        return false;
    };
    if rows.unsupported && rows.emitted == 0 {
        return false;
    }
    let stopped = state.stopped();
    match outcome {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) if rows.received == 0 => return false,
        Ok(Err(_)) | Err(_) if rows.received == 0 && !stopped => return false,
        _ if stopped => {}
        Ok(Ok(false)) => state.terminal_error(
            root,
            "Der Gegenseiten-Durchlauf meldete nach den ersten Einträgen „nicht unterstützt“; die Ansicht ist unvollständig."
                .to_string(),
        ),
        Ok(Err(error)) => state.terminal_error(
            root,
            format!("Der Gegenseiten-Durchlauf wurde unterbrochen; die Ansicht ist unvollständig: {error}"),
        ),
        Err(_) => state.terminal_error(
            root,
            "Der Gegenseiten-Durchlauf ist abgebrochen; die Ansicht ist unvollständig."
                .to_string(),
        ),
    }
    true
}

/// What the stream's items become: rows, failures, and the folders whose
/// content is not walked. Each item is decided as `WalkState::process_listing`
/// decides a listed entry.
struct Rows {
    root: String,
    /// The folder of the last row and its path, so siblings share one `Arc`.
    parent_cache: Option<(String, Arc<str>)>,
    /// Folders whose content is not walked (links, the app trash, folders the
    /// stream could not describe): nothing below them is listed.
    pruned: HashSet<String>,
    /// Items taken from the stream.
    received: u64,
    /// Rows emitted into the scan.
    emitted: u64,
    /// The side sent an entry without the metadata this request asked for.
    unsupported: bool,
}

impl Rows {
    fn new(root: &str) -> Self {
        Self {
            root: root.to_string(),
            parent_cache: None,
            pruned: HashSet::new(),
            received: 0,
            emitted: 0,
            unsupported: false,
        }
    }

    /// Takes one item. `false` ends the walk: a stop was requested, the scan
    /// budget is used up, or the side cannot send the metadata this walk needs.
    fn take(&mut self, state: &mut WalkState, item: HashWalkItem) -> bool {
        if state.stopped() {
            return false;
        }
        self.received = self.received.saturating_add(1);
        match item {
            HashWalkItem::Entry(entry) => self.entry(state, entry),
            HashWalkItem::Omitted(omitted) => {
                self.omitted(state, omitted);
                true
            }
        }
    }

    /// An entry that cannot be read or named: its own failure, at its path.
    fn omitted(&self, state: &mut WalkState, omitted: VfsOmission) {
        let path = join_rel(&self.root, &omitted.rel);
        state.omitted_path(&path, omitted.detail);
    }

    fn entry(&mut self, state: &mut WalkState, entry: HashWalkEntry) -> bool {
        let rel = entry.rel.as_str();
        let Some((parent_rel, name)) = split(rel) else {
            return true;
        };
        let depth = depth_of(rel);
        // Below a folder at the depth limit: that folder's failure is reported
        // with the folder itself.
        if depth > MAX_SCAN_DEPTH || self.below_pruned(rel) {
            return true;
        }
        let parent = self.parent_path(parent_rel);
        let path = join(&parent, name);
        let Some(meta) = entry.meta else {
            // The request asked for metadata. A side that sends none cannot
            // show links or hidden entries, so the caller lists folder by
            // folder, as long as no row has been emitted yet.
            if self.emitted == 0 {
                self.unsupported = true;
                return false;
            }
            state.omitted_path(
                &path,
                "Metadaten fehlen in der Antwort der Gegenseite".to_string(),
            );
            self.prune_if_dir(&entry);
            return true;
        };
        // The app trash is a protected omission: its content is not listed.
        if crate::apptrash::excluded_name(name) {
            self.prune_if_dir(&entry);
            return true;
        }
        if crate::vfs::validate_child_name(name).is_err() {
            state.omitted_path(&path, "Name ist kein Pfad".to_string());
            self.prune_if_dir(&entry);
            return true;
        }
        // A link is a row but is never walked into, as in the folder walk.
        if entry.is_dir && meta.is_symlink {
            self.pruned.insert(entry.rel.clone());
        }
        let is_dir = entry.is_dir;
        let size = entry.size;
        let limit_reached = is_dir && !meta.is_symlink && depth == MAX_SCAN_DEPTH;
        let row = FileEntry {
            path: Arc::from(path.as_str()),
            parent,
            name: Arc::from(name),
            ext: Arc::from(ext_of(name, is_dir).as_str()),
            size,
            mtime_ms: entry.mtime_ms,
            btime_ms: meta.btime_ms,
            is_dir,
            is_symlink: meta.is_symlink,
            hidden: meta.hidden,
            system: meta.system,
            depth,
            id: None,
        };
        state.count_entry(is_dir, size);
        if !state.emit(row) {
            return false;
        }
        self.emitted = self.emitted.saturating_add(1);
        if limit_reached {
            state.mark_truncated();
            state.listing_failed(
                &path,
                "Scan-Tiefenlimit erreicht; andere Ordner werden weiter gelesen",
            );
        }
        state.maybe_progress(&path)
    }

    fn prune_if_dir(&mut self, entry: &HashWalkEntry) {
        if entry.is_dir {
            self.pruned.insert(entry.rel.clone());
        }
    }

    /// Whether a proper ancestor folder of `rel` is pruned.
    fn below_pruned(&self, rel: &str) -> bool {
        if self.pruned.is_empty() {
            return false;
        }
        let mut rest = rel;
        while let Some(index) = rest.rfind('/') {
            rest = &rest[..index];
            if self.pruned.contains(rest) {
                return true;
            }
        }
        false
    }

    fn parent_path(&mut self, parent_rel: &str) -> Arc<str> {
        if let Some((cached, path)) = &self.parent_cache {
            if cached.as_str() == parent_rel {
                return path.clone();
            }
        }
        let text = if parent_rel.is_empty() {
            self.root.clone()
        } else {
            join(&self.root, parent_rel)
        };
        let path: Arc<str> = Arc::from(text.as_str());
        self.parent_cache = Some((parent_rel.to_string(), path.clone()));
        path
    }
}

/// The folder part and the name of a relative path; `None` for the root.
fn split(rel: &str) -> Option<(&str, &str)> {
    if rel.is_empty() {
        None
    } else {
        Some(rel.rsplit_once('/').unwrap_or(("", rel)))
    }
}

/// Folder depth of a relative path: its non-empty components.
fn depth_of(rel: &str) -> u32 {
    let count = rel.split('/').filter(|part| !part.is_empty()).count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn join_rel(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_string()
    } else {
        join(root, rel)
    }
}
