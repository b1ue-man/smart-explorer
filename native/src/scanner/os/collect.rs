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
    let mut outcome = CollectOutcome {
        entries: Vec::with_capacity(1024),
        ..CollectOutcome::default()
    };
    let root_link_metadata = match std::fs::symlink_metadata(root) {
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
        let read = match std::fs::read_dir(&directory) {
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
            let path = entry.path();
            let link_metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    push_issue(&mut outcome, &path, error.to_string());
                    continue;
                }
            };
            let is_symlink = is_link_like(&link_metadata);
            let metadata = if is_symlink && follow_symlinks {
                match std::fs::metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        push_issue(&mut outcome, &path, error.to_string());
                        continue;
                    }
                }
            } else {
                link_metadata
            };
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => {
                    push_issue(
                        &mut outcome,
                        &path,
                        "filename is not valid Unicode".to_string(),
                    );
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
            let is_dir = metadata.is_dir();
            let (hidden, system) = get_attrs(&metadata);
            let extension = ext_of(&name, is_dir);
            outcome.entries.push(FileEntry {
                path: Arc::from(path_text.as_str()),
                parent: parent.clone(),
                name: Arc::from(name.as_str()),
                ext: Arc::from(extension.as_str()),
                size: if is_dir { 0 } else { metadata.len() },
                mtime_ms: metadata.modified().map(ms_since_unix).unwrap_or(0),
                btime_ms: metadata.created().map(ms_since_unix).unwrap_or(0),
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
