use super::budget::MAX_SCAN_DEPTH;
use super::core::{ext_of, ms_since_unix};
use super::os::{record_failure, ScanMessage, Scanner};
use super::platform::{get_attrs, is_link_like, path_text};
use crate::types::FileEntry;
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

const BATCH_SIZE: usize = 1024;
const FLUSH_INTERVAL_MS: u128 = 60;

/// Walk directories in parallel while enforcing one shared memory/depth budget
/// and a visited-target set when following link-like directories.
pub(super) fn walk_parallel(scanner: &Arc<Scanner>, dirs: Vec<PathBuf>, depth: u32) {
    if dirs.is_empty() || scanner.cancel.load(Ordering::Relaxed) {
        return;
    }
    if depth > MAX_SCAN_DEPTH {
        let _ = scanner.claim_entry(0, depth, "scan depth");
        return;
    }

    dirs.into_par_iter().for_each(|dir| {
        if scanner.cancel.load(Ordering::Relaxed) || !scanner.enter_directory(&dir) {
            return;
        }
        let read = match std::fs::read_dir(&dir) {
            Ok(read) => read,
            Err(error) => {
                scanner.errors.fetch_add(1, Ordering::Relaxed);
                record_failure(
                    &scanner.failed_paths,
                    &dir.to_string_lossy(),
                    format!("read_dir: {error}"),
                );
                return;
            }
        };

        let mut batch = Vec::with_capacity(64);
        let mut subdirs = Vec::with_capacity(16);
        let mut last_flush = Instant::now();
        let Some(parent_text) = path_text(&dir) else {
            scanner.errors.fetch_add(1, Ordering::Relaxed);
            record_failure(
                &scanner.failed_paths,
                &format!("{dir:?}"),
                "directory path is not valid Unicode".to_string(),
            );
            return;
        };
        let parent: Arc<str> = Arc::from(parent_text.as_str());
        if let Ok(mut sample) = scanner.sample_path.try_lock() {
            *sample = parent_text.clone();
        }

        for entry in read {
            if scanner.cancel.load(Ordering::Relaxed) {
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    scanner.errors.fetch_add(1, Ordering::Relaxed);
                    record_failure(
                        &scanner.failed_paths,
                        &dir.to_string_lossy(),
                        format!("read_dir entry: {error}"),
                    );
                    continue;
                }
            };
            let path = entry.path();
            let link_metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    scanner.errors.fetch_add(1, Ordering::Relaxed);
                    record_failure(
                        &scanner.failed_paths,
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
                        scanner.errors.fetch_add(1, Ordering::Relaxed);
                        record_failure(
                            &scanner.failed_paths,
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
                    scanner.errors.fetch_add(1, Ordering::Relaxed);
                    record_failure(
                        &scanner.failed_paths,
                        &format!("{path:?}"),
                        "filename is not valid Unicode".to_string(),
                    );
                    continue;
                }
            };
            let extension = ext_of(&name, is_dir);
            let Some(path_text) = path_text(&path) else {
                scanner.errors.fetch_add(1, Ordering::Relaxed);
                record_failure(
                    &scanner.failed_paths,
                    &format!("{path:?}"),
                    "path is not valid Unicode".to_string(),
                );
                continue;
            };
            let retained_text = parent_text
                .len()
                .saturating_add(path_text.len())
                .saturating_add(name.len())
                .saturating_add(extension.len()) as u64;
            if !scanner.claim_entry(retained_text, depth, &path_text) {
                break;
            }

            let size = if is_dir { 0 } else { metadata.len() };
            batch.push(FileEntry {
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
            });
            scanner.scanned.fetch_add(1, Ordering::Relaxed);
            if !is_dir {
                scanner.bytes.fetch_add(size, Ordering::Relaxed);
            }

            if is_dir && (!is_symlink || scanner.opts.follow_symlinks) {
                let within_depth = scanner
                    .opts
                    .max_depth
                    .map_or(depth < MAX_SCAN_DEPTH, |maximum| depth < maximum);
                if within_depth {
                    subdirs.push(path);
                }
            }
            if batch.len() >= BATCH_SIZE || last_flush.elapsed().as_millis() > FLUSH_INTERVAL_MS {
                let chunk = std::mem::replace(&mut batch, Vec::with_capacity(64));
                if !scanner.send(ScanMessage::Entries(chunk)) {
                    break;
                }
                last_flush = Instant::now();
            }
        }

        if !batch.is_empty() {
            let _ = scanner.send(ScanMessage::Entries(batch));
        }
        if !subdirs.is_empty() {
            walk_parallel(scanner, subdirs, depth.saturating_add(1));
        }
    });
}
