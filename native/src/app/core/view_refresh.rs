//! M19: what a finished transfer changed in the current local view is read
//! again in the background and merged into the listing, instead of a full
//! `rescan()` of the whole view. Only the resolved roots (`Done.roots`) and,
//! for a move, the selected sources are read: a folder root with the depth
//! the view shows, a file (or a folder at the view's depth limit) as one row
//! of its parent's listing. Selection, cursor, sort order, collapsed folders
//! and scroll position stay; a vanished root leaves the view.
//!
//! Only a full rescan is correct (and is used) when the view itself or a
//! folder around it changed, the transfer resolved no roots, the view is
//! remote, a scan is running, or the listing was pruned with another filter
//! than a new scan would use.
use super::partial_scan::relative_depth;
use super::prelude::*;
use super::*;
use crate::scanner::RetentionHandle;
use crate::transfer::{JobItems, ResolvedRoot, TransferJob};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
#[path = "view_refresh_tests.rs"]
mod tests;

/// How often a re-read checks for cancellation while it waits on the scanner.
const WAIT_SLICE: std::time::Duration = std::time::Duration::from_millis(100);

/// One changed path below the view, spelled like the view's entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChangedRoot {
    pub(super) path: String,
    /// The folder whose listing holds this root's row.
    pub(super) parent: String,
    pub(super) name: String,
    /// Depth below the view's root (1 = a row of the view's own folder).
    pub(super) depth: u32,
}

/// What a re-read found for one root.
#[derive(Debug)]
pub(super) struct RootListing {
    /// The root in the scanner's spelling when it was found.
    pub(super) root: String,
    pub(super) entries: Vec<FileEntry>,
    /// Every row below `root` was read: rows not in `entries` are gone.
    pub(super) complete: bool,
}

/// Re-reads of the active view that still run.
#[derive(Default)]
pub(in crate::app) struct ViewRefresh {
    workers: Vec<RefreshWorker>,
}

impl ViewRefresh {
    /// A re-read runs; the frame loop keeps polling.
    pub(in crate::app) fn busy(&self) -> bool {
        !self.workers.is_empty()
    }
}

struct RefreshWorker {
    /// The view (`App::root_path`) and listing scope the re-read belongs to.
    root_path: String,
    max_depth: Option<u32>,
    pruned_with: Option<FilterDef>,
    rx: Receiver<Vec<RootListing>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for RefreshWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl App {
    /// Shows what finished transfer `id` changed by re-reading only those
    /// paths. False when only a full `rescan()` shows the change correctly;
    /// the caller then rescans as before.
    pub(in crate::app) fn refresh_transfer_roots(&mut self, id: u64) -> bool {
        if self.remote.is_some() || self.root_path.is_empty() || self.scan_running {
            return false;
        }
        let Some(paths) = self
            .transfer_center
            .finished_entry(id)
            .and_then(|entry| changed_paths(entry.job.as_deref()?, &entry.roots))
        else {
            return false;
        };
        let scope = self.scan_scope();
        if scope.pruned_with != self.scan_retention {
            return false;
        }
        let view = self.root_prefix();
        let Some(roots) = plan_roots(&self.root_path, &view, scope.max_depth, &paths) else {
            return false;
        };
        if roots.is_empty() {
            return true;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let (max_depth, retention) = (scope.max_depth, scope.retention);
        let spawned = std::thread::Builder::new()
            .name("view-refresh".into())
            .spawn(move || {
                let listings = read_roots(&roots, max_depth, retention.as_ref(), &stop);
                if !stop.load(Ordering::Acquire) {
                    let _ = tx.send(listings);
                }
            });
        if spawned.is_err() {
            return false;
        }
        self.view_refresh.workers.push(RefreshWorker {
            root_path: self.root_path.clone(),
            max_depth,
            pruned_with: scope.pruned_with,
            rx,
            cancel,
        });
        true
    }

    /// Merges finished re-reads; runs every frame with the scan drain.
    pub(in crate::app) fn poll_view_refresh(&mut self) {
        if self.view_refresh.workers.is_empty() {
            return;
        }
        let mut listings = Vec::new();
        let mut rescan = false;
        let mut running = Vec::new();
        for worker in std::mem::take(&mut self.view_refresh.workers) {
            // A scan started since, or another folder shown: the re-read is
            // moot (the scan sees the transfer's result) and is dropped.
            if self.scan_running || self.remote.is_some() || worker.root_path != self.root_path {
                continue;
            }
            match worker.rx.try_recv() {
                Ok(found) => {
                    let scope = self.scan_scope();
                    if scope.max_depth == worker.max_depth
                        && scope.pruned_with == worker.pruned_with
                        && self.scan_retention == worker.pruned_with
                    {
                        listings.extend(found);
                    } else {
                        rescan = true;
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => running.push(worker),
                // The re-read ended without a result.
                Err(crossbeam_channel::TryRecvError::Disconnected) => rescan = true,
            }
        }
        if rescan {
            self.rescan();
            return;
        }
        self.view_refresh.workers = running;
        if !listings.is_empty() {
            self.apply_root_listings(listings);
        }
    }

    /// Puts re-read roots into the listing; sort order and the still
    /// present selection, cursor and anchor stay.
    pub(super) fn apply_root_listings(&mut self, listings: Vec<RootListing>) {
        let (count, bytes) = merge_listings(&mut self.entries, listings);
        self.progress.scanned = shift(self.progress.scanned, count);
        self.progress.bytes = shift(self.progress.bytes, bytes);
        let keys: HashSet<Arc<str>> = self.entries.iter().map(FileEntry::key).collect();
        let gone = |key: &Option<Arc<str>>| key.as_ref().is_some_and(|key| !keys.contains(key));
        if gone(&self.cursor) {
            self.cursor = None;
        }
        if gone(&self.last_anchor) {
            self.last_anchor = None;
        }
        self.recompute_view();
    }
}

fn shift(value: u64, delta: i128) -> u64 {
    u64::try_from((i128::from(value) + delta).max(0)).unwrap_or(u64::MAX)
}

/// The local paths a finished job changed, in the job's spelling: the first
/// segment of every resolved root below the target folder (a new folder of
/// nested pairs included) and, for a move, every selected source. `None`
/// when the job resolved nothing at a local target or wrote into the
/// target folder itself.
pub(super) fn changed_paths(job: &TransferJob, roots: &[ResolvedRoot]) -> Option<Vec<String>> {
    let mut paths = Vec::new();
    if job.target.is_local() {
        if roots.is_empty() {
            return None;
        }
        for root in roots {
            let first = root.rel.split('/').find(|segment| !segment.is_empty())?;
            paths.push(join(&job.target_dir, &[first]));
        }
    }
    if job.mode == CopyMode::Move && job.source.is_local() {
        match &job.items {
            JobItems::Roots { paths: sources, .. } => paths.extend(sources.iter().cloned()),
            JobItems::Pairs(pairs) => paths.extend(pairs.iter().map(|pair| pair.source.clone())),
        }
    }
    Some(paths)
}

/// Path segments; views and jobs spell local paths with `/`.
fn segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn join(base: &str, tail: &[&str]) -> String {
    if tail.is_empty() {
        return base.to_string();
    }
    format!("{}/{}", base.trim_end_matches('/'), tail.join("/"))
}

/// The roots below view `root_path` (its entries spelled from `view`) that a
/// re-read must cover, outermost only and within `max_depth`. `None` when a
/// path is the view or holds it: then the whole view changed.
pub(super) fn plan_roots(
    root_path: &str,
    view: &str,
    max_depth: Option<u32>,
    paths: &[String],
) -> Option<Vec<ChangedRoot>> {
    let skip = segments(root_path).len();
    let mut found: Vec<ChangedRoot> = Vec::new();
    for path in paths {
        if crate::scanner::retry_contains(path, root_path) {
            return None;
        }
        let Some(depth) = relative_depth(root_path, path) else {
            continue;
        };
        if depth == 0 {
            return None;
        }
        if max_depth.is_some_and(|limit| depth > limit) {
            continue;
        }
        let tail = segments(path);
        // Another spelling than the view's (a backslash in a name): only a full
        // rescan is safe.
        let tail = tail
            .get(skip..)
            .filter(|tail| tail.len() == depth as usize)?;
        let (name, above) = tail.split_last()?;
        found.push(ChangedRoot {
            path: join(view, tail),
            parent: join(view, above),
            name: (*name).to_string(),
            depth,
        });
    }
    // Outermost first; a root inside another root is covered by it.
    found.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.path.cmp(&b.path)));
    let mut kept: HashSet<String> = HashSet::new();
    found.retain(|root| {
        !under_any(&root.path, |path| kept.contains(path)) && kept.insert(root.path.clone())
    });
    Some(found)
}

/// `path` or one of its ancestors is in the set.
fn under_any(path: &str, contains: impl Fn(&str) -> bool) -> bool {
    if contains(path) {
        return true;
    }
    let mut end = path.len();
    while let Some(cut) = path[..end].rfind('/') {
        if cut > 0 && contains(&path[..cut]) {
            return true;
        }
        end = cut;
    }
    false
}

fn local_path(path: &str) -> PathBuf {
    PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Reads every root the way the view's scan would list it.
pub(super) fn read_roots(
    roots: &[ChangedRoot],
    max_depth: Option<u32>,
    retention: Option<&RetentionHandle>,
    cancel: &AtomicBool,
) -> Vec<RootListing> {
    let mut listings = Vec::new();
    let mut rows: HashMap<&str, Vec<&ChangedRoot>> = HashMap::new();
    for root in roots {
        if cancel.load(Ordering::Acquire) {
            return listings;
        }
        let below = max_depth.map(|limit| limit.saturating_sub(root.depth));
        match crate::local_access::symlink_metadata(&local_path(&root.path)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                listings.push(RootListing {
                    root: root.path.clone(),
                    entries: Vec::new(),
                    complete: true,
                });
            }
            // Unreadable now: the view keeps what it shows.
            Err(_) => {}
            // A folder the view shows the inside of: its own scan.
            Ok(metadata) if metadata.is_dir() && below != Some(0) => {
                let Some((entries, complete)) =
                    scan(&root.path, root.depth, below, retention, cancel)
                else {
                    return listings;
                };
                let spelled = entries
                    .iter()
                    .find(|entry| entry.depth == root.depth)
                    .map_or_else(|| root.path.clone(), |entry| entry.path.to_string());
                listings.push(RootListing {
                    root: spelled,
                    entries,
                    complete,
                });
            }
            // A file, a link or a folder at the depth limit: one row.
            Ok(_) => rows.entry(root.parent.as_str()).or_default().push(root),
        }
    }
    for (parent, group) in rows {
        let depth = group[0].depth.saturating_sub(1);
        let Some((entries, complete)) = scan(parent, depth, Some(1), retention, cancel) else {
            return listings;
        };
        let by_name: HashMap<&str, &FileEntry> = entries
            .iter()
            .filter(|entry| entry.depth == depth + 1)
            .map(|entry| (entry.name.as_ref(), entry))
            .collect();
        for root in group {
            let found = by_name.get(root.name.as_str()).copied().cloned();
            listings.push(RootListing {
                root: found
                    .as_ref()
                    .map_or_else(|| root.path.clone(), |entry| entry.path.to_string()),
                complete: complete || found.is_some(),
                entries: found.into_iter().collect(),
            });
        }
    }
    listings
}

/// Scans `path` (at `depth` below the view) to `max_depth` levels; `None`
/// once canceled. The flag tells whether every entry was read.
fn scan(
    path: &str,
    depth: u32,
    max_depth: Option<u32>,
    retention: Option<&RetentionHandle>,
    cancel: &AtomicBool,
) -> Option<(Vec<FileEntry>, bool)> {
    let (tx, rx) = unbounded();
    let handle = start_scan(
        local_path(path),
        ScanOpts {
            follow_symlinks: false,
            max_depth,
            retention: retention.cloned(),
        },
        tx,
    );
    let mut entries = Vec::new();
    loop {
        if cancel.load(Ordering::Acquire) {
            handle.cancel.store(true, Ordering::Release);
            return None;
        }
        match rx.recv_timeout(WAIT_SLICE) {
            Ok(ScanMessage::Entries(mut batch)) => {
                for entry in &mut batch {
                    entry.depth = entry.depth.saturating_add(depth);
                }
                entries.append(&mut batch);
            }
            Ok(ScanMessage::Done(progress)) => {
                let truncated = handle.truncated.load(Ordering::Acquire);
                return Some((entries, progress.errors == 0 && !truncated));
            }
            Ok(_) => {}
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                return Some((entries, false));
            }
        }
    }
}

/// Merges all listings in one pass: a complete root replaces its old
/// subtree, an incomplete one only adds or updates rows. Returns the change
/// of row count and bytes below the merged roots.
pub(super) fn merge_listings(
    entries: &mut Vec<FileEntry>,
    listings: Vec<RootListing>,
) -> (i128, i128) {
    let all: HashSet<String> = listings.iter().map(|l| l.root.clone()).collect();
    let before = measure(entries, &all);
    let replaced: HashSet<&str> = listings
        .iter()
        .filter(|listing| listing.complete)
        .map(|listing| listing.root.as_str())
        .collect();
    if !replaced.is_empty() {
        entries.retain(|entry| !under_any(&entry.path, |path| replaced.contains(path)));
    }
    let mut positions: HashMap<Arc<str>, usize> = if listings.iter().all(|l| l.complete) {
        HashMap::new()
    } else {
        entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.path.clone(), index))
            .collect()
    };
    for entry in listings.into_iter().flat_map(|listing| listing.entries) {
        match positions.get(&entry.path) {
            Some(&index) => entries[index] = entry,
            None => {
                positions.insert(entry.path.clone(), entries.len());
                entries.push(entry);
            }
        }
    }
    let after = measure(entries, &all);
    (after.0 - before.0, after.1 - before.1)
}

/// Rows and bytes at or below `roots`.
fn measure(entries: &[FileEntry], roots: &HashSet<String>) -> (i128, i128) {
    let mut total = (0, 0);
    for entry in entries {
        if under_any(&entry.path, |path| roots.contains(path)) {
            total.0 += 1;
            total.1 += i128::from(entry.size);
        }
    }
    total
}
