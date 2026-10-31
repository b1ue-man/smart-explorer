//! The walk of a copy expansion: folders in stack order (depth first, the newest
//! folder first). A folder's listing comes from a listing thread when one was
//! read ahead (`collect_listing.rs`), otherwise the walk reads it at its turn.
//! Entries, messages, budgets and access requests follow this order.
use super::super::core::{ext_of, ms_since_unix};
use super::super::platform::{get_attrs, is_link_like, path_text};
use super::listing;
use super::{push_issue, CollectOutcome, Limits, ReadGrant};
use crate::source_media::WalkDevice;
use crate::types::FileEntry;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A folder the walk has found and has not read yet.
struct Pending {
    directory: PathBuf,
    depth: u32,
    relative_depth: u32,
    /// The device of the folder it was entered from; `None` for the root.
    parent: Option<WalkDevice>,
    /// Its listing, read ahead by a listing thread when one was asked for.
    ahead: Option<listing::Ahead>,
}

pub(super) fn walk(
    root: &Path,
    follow_symlinks: bool,
    start_depth: u32,
    cancel: &AtomicBool,
    mut grant: Option<&dyn ReadGrant>,
    limits: Limits,
    listers: &listing::Listers<'_>,
) -> CollectOutcome {
    let normalized = crate::local_access::normalize_scan_root(root);
    let root = normalized.as_path();
    let mut outcome = CollectOutcome {
        entries: Vec::with_capacity(1024),
        ..CollectOutcome::default()
    };
    let root_link_metadata =
        match crate::source_media::acquire(root, crate::source_media::Priority::Foreground, cancel)
            .and_then(|_guard| crate::local_access::symlink_metadata(root))
        {
            Ok(metadata) => metadata,
            Err(error) => {
                push_issue(&mut outcome, root, error.to_string(), limits.issues);
                return outcome;
            }
        };
    let root_is_link = is_link_like(&root_link_metadata);
    let root_metadata = if root_is_link && follow_symlinks {
        match crate::source_media::acquire(root, crate::source_media::Priority::Foreground, cancel)
            .and_then(|_guard| std::fs::metadata(root))
        {
            Ok(metadata) => metadata,
            Err(error) => {
                push_issue(&mut outcome, root, error.to_string(), limits.issues);
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
            limits.issues,
        );
        return outcome;
    }

    let optical = match crate::source_media::probe(root) {
        Ok(source) => source,
        Err(error) => {
            push_issue(&mut outcome, root, error.to_string(), limits.issues);
            return outcome;
        }
    };
    let mut name_bytes = 0usize;
    let mut stack = vec![Pending {
        directory: root.to_path_buf(),
        depth: start_depth,
        relative_depth: 0,
        parent: None,
        ahead: None,
    }];
    read_ahead(listers, &mut stack, limits);
    while let Some(pending) = stack.pop() {
        // Taking the top folder moves the window of read-ahead listings down.
        read_ahead(listers, &mut stack, limits);
        let Pending {
            directory,
            depth,
            relative_depth,
            parent,
            ahead,
        } = pending;
        if cancel.load(Ordering::Relaxed) {
            outcome.canceled = true;
            break;
        }
        if optical.as_ref().is_some_and(|expected| {
            !crate::source_media::probe(root).is_ok_and(|source| source.as_ref() == Some(expected))
        }) {
            push_issue(
                &mut outcome,
                root,
                "optical medium changed during copy expansion".into(),
                limits.issues,
            );
            break;
        }
        if relative_depth > limits.depth {
            push_issue(
                &mut outcome,
                &directory,
                format!("copy expansion exceeds {} levels", limits.depth),
                limits.issues,
            );
            break;
        }
        let read = match ahead {
            Some(ahead) => listers.claim(ahead),
            None => listing::Read::Reread,
        };
        if cancel.load(Ordering::Relaxed) {
            outcome.canceled = true;
            break;
        }
        let mut listed = match read {
            listing::Read::Listed(device, entries) => {
                let items: listing::Items<'_> = Box::new(entries.into_iter());
                Ok((device, items))
            }
            listing::Read::Reread => {
                listing::open(&directory, parent.as_ref(), follow_symlinks, cancel)
            }
        };
        if listed
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::PermissionDenied)
        {
            if let Some(access) = grant.take() {
                let requested = crate::local_access::display_path(root);
                if access.can_request(&requested) {
                    match access.request(&requested) {
                        Ok(true) => {
                            listed =
                                listing::open(&directory, parent.as_ref(), follow_symlinks, cancel)
                        }
                        Ok(false) => {
                            outcome.canceled = true;
                            return outcome;
                        }
                        Err(error) => {
                            push_issue(&mut outcome, &directory, error, limits.issues);
                            return outcome;
                        }
                    }
                }
            }
        }
        let (device, read) = match listed {
            Ok(pair) => pair,
            Err(error) => {
                push_issue(&mut outcome, &directory, error.to_string(), limits.issues);
                continue;
            }
        };
        let parent_text: Arc<str> = match path_text(&directory) {
            Some(text) => Arc::from(text),
            None => {
                push_issue(
                    &mut outcome,
                    &directory,
                    "path is not valid Unicode".to_string(),
                    limits.issues,
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
                    push_issue(&mut outcome, &directory, error.to_string(), limits.issues);
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
                    limits.issues,
                );
                continue;
            }
            let is_symlink =
                entry.is_link_like || entry.kind == crate::local_access::EntryKind::Link;
            let (is_dir, hidden, system, size, mtime_ms, btime_ms) =
                if is_symlink && follow_symlinks {
                    let metadata = match crate::source_media::acquire(
                        &path,
                        crate::source_media::Priority::Foreground,
                        cancel,
                    )
                    .and_then(|_guard| std::fs::metadata(&path))
                    {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            push_issue(&mut outcome, &path, error.to_string(), limits.issues);
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
                    push_issue(
                        &mut outcome,
                        &path,
                        "filename is not valid Unicode".into(),
                        limits.issues,
                    );
                    continue;
                }
            };
            let path_text = match path_text(&path) {
                Some(path) => path,
                None => {
                    push_issue(
                        &mut outcome,
                        &path,
                        "path is not valid Unicode".to_string(),
                        limits.issues,
                    );
                    continue;
                }
            };
            name_bytes = name_bytes
                .saturating_add(name.len())
                .saturating_add(path_text.len());
            if outcome.entries.len() >= limits.entries || name_bytes > limits.name_bytes {
                push_issue(
                    &mut outcome,
                    &path,
                    format!(
                        "copy expansion exceeds its limit of {} entries or {} MiB of names",
                        limits.entries,
                        limits.name_bytes / (1024 * 1024)
                    ),
                    limits.issues,
                );
                return outcome;
            }
            let extension = ext_of(&name, is_dir);
            outcome.entries.push(FileEntry {
                path: Arc::from(path_text.as_str()),
                parent: Arc::clone(&parent_text),
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
                stack.push(Pending {
                    directory: path,
                    depth: depth.saturating_add(1),
                    relative_depth: relative_depth.saturating_add(1),
                    parent: Some(device.clone()),
                    ahead: None,
                });
                read_ahead(listers, &mut stack, limits);
            }
        }
        if outcome.canceled {
            break;
        }
    }
    if optical.as_ref().is_some_and(|expected| {
        !crate::source_media::probe(root).is_ok_and(|source| source.as_ref() == Some(expected))
    }) {
        push_issue(
            &mut outcome,
            root,
            "optical medium changed during copy expansion".into(),
            limits.issues,
        );
    }
    outcome
}

/// Asks for read-ahead listings of the folders nearest the top of the stack:
/// the folders the walk takes next.
fn read_ahead(listers: &listing::Listers<'_>, stack: &mut [Pending], limits: Limits) {
    for pending in stack.iter_mut().rev().take(listers.window()) {
        if pending.ahead.is_some() || pending.relative_depth > limits.depth {
            continue;
        }
        match listers.read_ahead(&pending.directory, pending.parent.as_ref()) {
            Some(ahead) => pending.ahead = Some(ahead),
            None => break,
        }
    }
}
