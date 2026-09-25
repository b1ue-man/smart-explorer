use super::core::{ext_of, ms_since_unix};
use super::platform::{get_attrs, is_link_like, path_text};
use crate::types::FileEntry;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const MAX_COLLECTED_ENTRIES: usize = 1_000_000;
const MAX_COLLECTED_NAME_BYTES: usize = 128 * 1024 * 1024;
const MAX_COLLECTED_DEPTH: u32 = 512;
const MAX_COLLECT_ISSUES: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectIssue {
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct CollectOutcome {
    pub entries: Vec<FileEntry>,
    pub issues: Vec<CollectIssue>,
    pub suppressed_issues: u64,
    pub canceled: bool,
    /// Entries left out as protected omissions (the active app trash on
    /// Android); they never make the outcome incomplete. Always 0 on the
    /// desktop builds.
    pub omitted: u64,
}

impl CollectOutcome {
    pub fn is_complete(&self) -> bool {
        !self.canceled && self.issues.is_empty() && self.suppressed_issues == 0
    }
}

/// Collect a copy-selection subtree with explicit cancellation, bounded memory,
/// and all-or-nothing completeness reporting. Callers must not apply a move
/// from `entries` unless `is_complete()` is true.
pub fn collect_recursive(
    root: &Path,
    follow_symlinks: bool,
    start_depth: u32,
    cancel: &AtomicBool,
) -> CollectOutcome {
    collect(root, follow_symlinks, start_depth, cancel, false)
}

/// Explicit copy preparation may request one scoped read grant. Background
/// discovery uses `collect_recursive` and never opens a consent dialog.
pub fn collect_recursive_with_access(
    root: &Path,
    follow_symlinks: bool,
    start_depth: u32,
    cancel: &AtomicBool,
) -> CollectOutcome {
    collect(root, follow_symlinks, start_depth, cancel, true)
}

fn collect(
    root: &Path,
    follow_symlinks: bool,
    start_depth: u32,
    cancel: &AtomicBool,
    mut may_request: bool,
) -> CollectOutcome {
    let normalized = crate::local_access::normalize_scan_root(root);
    let root = normalized.as_path();
    let mut outcome = CollectOutcome {
        entries: Vec::with_capacity(1024),
        ..CollectOutcome::default()
    };
    let root_link_metadata = match crate::local_access::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) => {
            push_issue(&mut outcome, root, error.to_string());
            return outcome;
        }
    };
    let root_is_link = is_link_like(&root_link_metadata);
    let root_metadata = if root_is_link && follow_symlinks {
        match std::fs::metadata(root) {
            Ok(metadata) => metadata,
            Err(error) => {
                push_issue(&mut outcome, root, error.to_string());
                return outcome;
            }
        }
    } else {
        root_link_metadata
    };
    if !root_metadata.is_dir() || (root_is_link && !follow_symlinks) {
        push_issue(
            &mut outcome,
            root,
            "copy expansion root is not a traversable directory".to_string(),
        );
        return outcome;
    }

    let mut name_bytes = 0usize;
    let mut stack = vec![(root.to_path_buf(), start_depth, 0u32)];
    while let Some((directory, depth, relative_depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            outcome.canceled = true;
            break;
        }
        if relative_depth > MAX_COLLECTED_DEPTH {
            push_issue(
                &mut outcome,
                &directory,
                format!("copy expansion exceeds {MAX_COLLECTED_DEPTH} levels"),
            );
            break;
        }
        let mut listing = crate::local_access::read_directory(&directory);
        if may_request
            && listing
                .as_ref()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
        {
            may_request = false;
            let requested = crate::local_access::display_path(root);
            if crate::local_access::can_request_access(&requested) {
                match crate::local_access::request_access(&requested) {
                    Ok(true) => listing = crate::local_access::read_directory(&directory),
                    Ok(false) => {
                        outcome.canceled = true;
                        return outcome;
                    }
                    Err(error) => {
                        push_issue(&mut outcome, &directory, error);
                        return outcome;
                    }
                }
            }
        }
        let read = match listing {
            Ok(read) => read,
            Err(error) => {
                push_issue(&mut outcome, &directory, error.to_string());
                continue;
            }
        };
        let parent = match path_text(&directory) {
            Some(parent) => Arc::<str>::from(parent),
            None => {
                push_issue(
                    &mut outcome,
                    &directory,
                    "path is not valid Unicode".to_string(),
                );
                continue;
            }
        };
        for entry in read {
            if cancel.load(Ordering::Relaxed) {
                outcome.canceled = true;
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    push_issue(&mut outcome, &directory, error.to_string());
                    continue;
                }
            };
            if entry
                .name
                .to_str()
                .is_some_and(crate::apptrash::excluded_name)
            {
                outcome.omitted = outcome.omitted.saturating_add(1);
                continue;
            }
            let path = directory.join(&entry.name);
            if entry.unreachable {
                push_issue(
                    &mut outcome,
                    &path,
                    "Name ist kein darstellbarer Dateipfad".into(),
                );
                continue;
            }
            let is_symlink =
                entry.is_link_like || entry.kind == crate::local_access::EntryKind::Link;
            let (is_dir, hidden, system, size, mtime_ms, btime_ms) =
                if is_symlink && follow_symlinks {
                    let metadata = match std::fs::metadata(&path) {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            push_issue(&mut outcome, &path, error.to_string());
                            continue;
                        }
                    };
                    let (hidden, system) = get_attrs(&metadata);
                    (
                        metadata.is_dir(),
                        hidden,
                        system,
                        metadata.len(),
                        metadata.modified().map(ms_since_unix).unwrap_or(0),
                        metadata.created().map(ms_since_unix).unwrap_or(0),
                    )
                } else {
                    (
                        entry.is_dir,
                        entry.hidden,
                        entry.system,
                        entry.size,
                        entry.mtime_ms,
                        entry.btime_ms,
                    )
                };
            let name = match entry.name.into_string() {
                Ok(name) => name,
                Err(_) => {
                    push_issue(&mut outcome, &path, "filename is not valid Unicode".into());
                    continue;
                }
            };
            let path_text = match path_text(&path) {
                Some(path) => path,
                None => {
                    push_issue(&mut outcome, &path, "path is not valid Unicode".to_string());
                    continue;
                }
            };
            name_bytes = name_bytes
                .saturating_add(name.len())
                .saturating_add(path_text.len());
            if outcome.entries.len() >= MAX_COLLECTED_ENTRIES
                || name_bytes > MAX_COLLECTED_NAME_BYTES
            {
                push_issue(
                    &mut outcome,
                    &path,
                    format!(
                        "copy expansion exceeds its limit of {MAX_COLLECTED_ENTRIES} entries or {} MiB of names",
                        MAX_COLLECTED_NAME_BYTES / (1024 * 1024)
                    ),
                );
                return outcome;
            }
            let extension = ext_of(&name, is_dir);
            outcome.entries.push(FileEntry {
                path: Arc::from(path_text.as_str()),
                parent: parent.clone(),
                name: Arc::from(name.as_str()),
                ext: Arc::from(extension.as_str()),
                size: if is_dir { 0 } else { size },
                mtime_ms,
                btime_ms,
                is_dir,
                is_symlink,
                hidden,
                system,
                depth,
                id: None,
            });
            if is_dir && (!is_symlink || follow_symlinks) {
                stack.push((
                    path,
                    depth.saturating_add(1),
                    relative_depth.saturating_add(1),
                ));
            }
        }
        if outcome.canceled {
            break;
        }
    }
    outcome
}

fn push_issue(outcome: &mut CollectOutcome, path: &Path, detail: String) {
    if outcome.issues.len() < MAX_COLLECT_ISSUES {
        outcome.issues.push(CollectIssue {
            path: path.to_string_lossy().into_owned(),
            detail,
        });
    } else {
        outcome.suppressed_issues = outcome.suppressed_issues.saturating_add(1);
    }
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
