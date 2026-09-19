use super::budget::MAX_SCAN_DEPTH;
use super::core::{ext_of, ms_since_unix};
use super::os::{record_failure, ScanMessage, Scanner};
use super::platform::{get_attrs, is_link_like, path_text};
use super::retention::Lineage;
use crate::types::FileEntry;
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

const BATCH_SIZE: usize = 1024;
const FLUSH_INTERVAL_MS: u128 = 60;

/// A directory to list. `lineage` links the ancestors whose entries were not
/// emitted yet (only while a retention filter is active); `None` means every
/// ancestor is already known downstream.
pub(super) struct PendingDir {
    pub(super) path: PathBuf,
    pub(super) lineage: Option<Arc<Lineage>>,
}

/// Walk directories in parallel while enforcing one shared memory/depth budget
/// and a visited-target set when following link-like directories.
pub(super) fn walk_parallel(scanner: &Arc<Scanner>, dirs: Vec<PendingDir>, depth: u32) {
    if dirs.is_empty() || scanner.cancel.load(Ordering::Relaxed) {
        return;
    }
    if depth > MAX_SCAN_DEPTH {
        let _ = scanner.claim_entry(0, depth, "scan depth");
        return;
    }

    dirs.into_par_iter().for_each(|dir| {
        if scanner.cancel.load(Ordering::Relaxed) || !scanner.enter_directory(&dir.path) {
            return;
        }
        let subdirs = list_directory(scanner, dir, depth);
        if !subdirs.is_empty() {
            walk_parallel(scanner, subdirs, depth.saturating_add(1));
        }
    });
}

/// List one directory, emit what the retention policy keeps and return the
/// subdirectories to descend into.
fn list_directory(scanner: &Arc<Scanner>, dir: PendingDir, depth: u32) -> Vec<PendingDir> {
    let read = match std::fs::read_dir(&dir.path) {
        Ok(read) => read,
        Err(error) => {
            fail(
                scanner,
                &dir.path.to_string_lossy(),
                format!("read_dir: {error}"),
            );
            return Vec::new();
        }
    };
    let Some(parent_text) = path_text(&dir.path) else {
        fail(
            scanner,
            &format!("{:?}", dir.path),
            "directory path is not valid Unicode".to_string(),
        );
        return Vec::new();
    };
    let parent: Arc<str> = Arc::from(parent_text.as_str());
    if let Ok(mut sample) = scanner.sample_path.try_lock() {
        *sample = parent_text;
    }
    let within_depth = scanner
        .opts
        .max_depth
        .map_or(depth < MAX_SCAN_DEPTH, |maximum| depth < maximum);

    let mut sink = BatchSink::new(scanner);
    let mut subdirs = Vec::with_capacity(16);
    for entry in read {
        if scanner.cancel.load(Ordering::Relaxed) {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                fail(
                    scanner,
                    &dir.path.to_string_lossy(),
                    format!("read_dir entry: {error}"),
                );
                continue;
            }
        };
        let path = entry.path();
        // The enumeration record carries the entry's own (non-following)
        // metadata. Re-opening `path` would let Win32 resolve a reserved name
        // such as `NUL` to the device instead of the stored file.
        let link_metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                fail(
                    scanner,
                    &path.to_string_lossy(),
                    format!("metadata: {error}"),
                );
                continue;
            }
        };
        let is_symlink = is_link_like(&link_metadata);
        let metadata = if is_symlink && scanner.opts.follow_symlinks {
            match std::fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    fail(
                        scanner,
                        &path.to_string_lossy(),
                        format!("follow metadata: {error}"),
                    );
                    continue;
                }
            }
        } else {
            link_metadata
        };
        let is_dir = metadata.is_dir();
        let (hidden, system) = get_attrs(&metadata);
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => {
                fail(
                    scanner,
                    &format!("{path:?}"),
                    "filename is not valid Unicode".to_string(),
                );
                continue;
            }
        };
        let extension = ext_of(&name, is_dir);
        let Some(path_text) = path_text(&path) else {
            fail(
                scanner,
                &format!("{path:?}"),
                "path is not valid Unicode".to_string(),
            );
            continue;
        };

        let size = if is_dir { 0 } else { metadata.len() };
        let file_entry = FileEntry {
            path: Arc::from(path_text.as_str()),
            parent: parent.clone(),
            name: Arc::from(name.as_str()),
            ext: Arc::from(extension.as_str()),
            size,
            mtime_ms: metadata.modified().map(ms_since_unix).unwrap_or(0),
            btime_ms: metadata.created().map(ms_since_unix).unwrap_or(0),
            is_dir,
            is_symlink,
            hidden,
            system,
            depth,
            id: None,
        };
        scanner.scanned.fetch_add(1, Ordering::Relaxed);
        if !is_dir {
            scanner.bytes.fetch_add(size, Ordering::Relaxed);
        }
        let traversable = is_dir && (!is_symlink || scanner.opts.follow_symlinks) && within_depth;
        if is_dir && !is_symlink && !within_depth && scanner.opts.max_depth.is_none() {
            scanner.truncated.store(true, Ordering::Relaxed);
            fail(scanner, &path_text, "Scan-Tiefenlimit erreicht; andere Ordner werden weiter gelesen".into());
        }

        match scanner.opts.retention.as_ref() {
            None => {
                if !sink.emit(file_entry) {
                    break;
                }
                if traversable {
                    subdirs.push(PendingDir {
                        path,
                        lineage: None,
                    });
                }
            }
            Some(retention) => {
                let keep = retention.retain(&file_entry);
                let descend = traversable && retention.descend(&file_entry);
                if keep && (!sink.emit_lineage(&dir.lineage) || !sink.emit(file_entry.clone())) {
                    break;
                }
                if descend {
                    // A kept directory and its ancestors are emitted already;
                    // otherwise the children inherit it as a pending ancestor.
                    let lineage =
                        (!keep).then(|| Lineage::pending(file_entry, dir.lineage.clone()));
                    subdirs.push(PendingDir { path, lineage });
                }
            }
        }
    }
    sink.flush();
    if scanner.cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }
    subdirs
}

fn fail(scanner: &Scanner, path: &str, detail: String) {
    scanner.errors.fetch_add(1, Ordering::Relaxed);
    record_failure(&scanner.failed_paths, path, detail);
}

/// Entries emitted by one directory listing, flushed in bounded batches. Every
/// emitted entry claims the shared budget first.
struct BatchSink<'a> {
    scanner: &'a Arc<Scanner>,
    entries: Vec<FileEntry>,
    last_flush: Instant,
}

impl<'a> BatchSink<'a> {
    fn new(scanner: &'a Arc<Scanner>) -> Self {
        Self {
            scanner,
            entries: Vec::with_capacity(64),
            last_flush: Instant::now(),
        }
    }

    fn emit(&mut self, entry: FileEntry) -> bool {
        let retained_text = entry
            .parent
            .len()
            .saturating_add(entry.path.len())
            .saturating_add(entry.name.len())
            .saturating_add(entry.ext.len()) as u64;
        if !self
            .scanner
            .claim_entry(retained_text, entry.depth, &entry.path)
        {
            return false;
        }
        self.entries.push(entry);
        if self.entries.len() >= BATCH_SIZE
            || self.last_flush.elapsed().as_millis() > FLUSH_INTERVAL_MS
        {
            return self.flush();
        }
        true
    }

    /// Emit the not-yet-emitted ancestors of the current directory so a kept
    /// entry has a place in the tree.
    fn emit_lineage(&mut self, lineage: &Option<Arc<Lineage>>) -> bool {
        Lineage::emit_pending(lineage, |entry| self.emit(entry.clone()))
    }

    fn flush(&mut self) -> bool {
        if self.entries.is_empty() {
            return true;
        }
        let chunk = std::mem::replace(&mut self.entries, Vec::with_capacity(64));
        self.last_flush = Instant::now();
        self.scanner.send(ScanMessage::Entries(chunk))
    }
}

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
