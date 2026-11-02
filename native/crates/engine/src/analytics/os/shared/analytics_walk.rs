//! Directory assembly of the size scanner; handles confine peer scans.
use super::*;
use std::{cmp::Reverse, collections::BinaryHeap};

pub(super) fn scan_entries(
    traversal: &Traversal<'_>,
    dir: &Path,
    name: Box<str>,
    entries: io::Result<impl Iterator<Item = io::Result<LocalEntry>>>,
    depth: u32,
    is_root: bool,
) -> SizeNode {
    scan_entries_with(traversal, dir, name, entries, depth, is_root, None, &[])
}

pub(super) fn scan_entries_with(
    traversal: &Traversal<'_>,
    dir: &Path,
    name: Box<str>,
    entries: io::Result<impl Iterator<Item = io::Result<LocalEntry>>>,
    depth: u32,
    is_root: bool,
    handle: Option<&crate::local_access::DirectoryHandle>,
    excluded: &[PathBuf],
) -> SizeNode {
    let p = traversal.progress;
    let diagnostics = traversal.diagnostics;
    let budget = traversal.budget;
    let mut subdirs: Vec<(PathBuf, Box<str>, Retention)> = Vec::new();
    // The weakest retained file is on top: lower size, then later name.
    // Enumeration never keeps all names of a million-file directory.
    let mut files: BinaryHeap<(Reverse<u64>, Box<str>, u64)> = BinaryHeap::new();
    let mut own_files = 0u64;
    let mut own_bytes = 0u64;
    let mut aggregated_bytes = 0u64;
    let mut aggregated_entries = 0u64;
    let mut subtree_bytes = 0u64;
    let mut dir_nodes = Vec::new();
    let mut reported_files = 0u64;
    let mut reported_bytes = 0u64;

    match entries {
        Ok(rd) => {
            for entry in rd {
                let ent = match entry {
                    Ok(ent) => ent,
                    Err(error) => {
                        diagnostics.entry_failed(dir, &error);
                        continue;
                    }
                };
                if p.cancel.load(Ordering::Relaxed) {
                    break;
                }
                if ent.is_link_like || matches!(ent.kind, EntryKind::Link | EntryKind::Other) {
                    continue;
                }
                let nm: Box<str> = ent.name.to_string_lossy().into_owned().into_boxed_str();
                // The app trash (Android) is left out like in every other walk.
                if crate::apptrash::excluded_name(&nm)
                    || crate::bisync::is_engine_name(&nm)
                    || crate::vfs::is_staging_name(&nm)
                    || excluded.iter().any(|hidden| hidden == &dir.join(&ent.name))
                {
                    continue;
                }
                if ent.kind == EntryKind::Directory {
                    if ent.unreachable {
                        diagnostics.record(
                            format!(
                                "{}{}{}",
                                crate::analytics::os::display_path(dir),
                                std::path::MAIN_SEPARATOR,
                                nm
                            ),
                            "Ordnername ist nicht als Pfad darstellbar; Inhalt nicht erfasst",
                            false,
                        );
                        continue;
                    }
                    let path = dir.join(&ent.name);
                    if crate::agent_proto::is_pseudo_dir(&path.to_string_lossy()) {
                        continue; // /proc, /sys, … report bogus huge sizes
                    }
                    let retention =
                        budget.claim(&path, depth.saturating_add(1), nm.len() as u64, diagnostics);
                    subdirs.push((path, nm, retention));
                    if subdirs.len() == 256 {
                        p.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);
                        fold_children(
                            scan_children(
                                traversal,
                                std::mem::take(&mut subdirs),
                                depth,
                                handle,
                                excluded,
                            ),
                            &mut dir_nodes,
                            &mut subtree_bytes,
                            &mut aggregated_bytes,
                            &mut aggregated_entries,
                        );
                        p.enter_directory(&crate::analytics::os::display_path(dir));
                    }
                } else if ent.kind == EntryKind::File {
                    own_files += 1;
                    own_bytes = own_bytes.saturating_add(ent.size);
                    let keep = files.len() < MAX_RETAINED_FILES_PER_DIRECTORY
                        || files.peek().is_some_and(|(size, name, _)| {
                            ent.size > size.0 || (ent.size == size.0 && nm.as_ref() < name.as_ref())
                        });
                    if keep {
                        if files.len() == MAX_RETAINED_FILES_PER_DIRECTORY {
                            if let Some((size, _, _)) = files.pop() {
                                aggregated_bytes = aggregated_bytes.saturating_add(size.0);
                                aggregated_entries += 1;
                            }
                        }
                        files.push((Reverse(ent.size), nm, own_files));
                    } else {
                        aggregated_bytes = aggregated_bytes.saturating_add(ent.size);
                        aggregated_entries += 1;
                    }
                    if own_files - reported_files >= 128 {
                        p.files
                            .fetch_add(own_files - reported_files, Ordering::Relaxed);
                        p.bytes
                            .fetch_add(own_bytes - reported_bytes, Ordering::Relaxed);
                        reported_files = own_files;
                        reported_bytes = own_bytes;
                    }
                }
            }
        }
        Err(error) => diagnostics.dir_failed(dir, &error, is_root),
    }

    p.files
        .fetch_add(own_files - reported_files, Ordering::Relaxed);
    p.bytes
        .fetch_add(own_bytes - reported_bytes, Ordering::Relaxed);
    p.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);

    // Retain the largest files individually; fold the rest of a huge
    // directory into one aggregate node so totals stay exact.
    let mut files = files.into_vec();
    if own_files > MAX_RETAINED_FILES_PER_DIRECTORY as u64 {
        files.sort_by(|left, right| {
            right
                .0
                 .0
                .cmp(&left.0 .0)
                .then_with(|| left.1.cmp(&right.1))
        });
    } else {
        files.sort_by_key(|file| file.2);
    }
    let mut file_nodes: Vec<SizeNode> =
        Vec::with_capacity(files.len().min(MAX_RETAINED_FILES_PER_DIRECTORY));
    for (Reverse(size), file_name, _) in files {
        let retained = budget.claim(
            &dir.join(&*file_name),
            depth.saturating_add(1),
            file_name.len() as u64,
            diagnostics,
        ) == Retention::Keep;
        if retained {
            file_nodes.push(SizeNode {
                name: file_name,
                size,
                is_dir: false,
                children: Vec::new(),
            });
        } else {
            aggregated_bytes = aggregated_bytes.saturating_add(size);
            aggregated_entries += 1;
        }
    }
    diagnostics.count_aggregated_files(own_files.saturating_sub(file_nodes.len() as u64));

    fold_children(
        scan_children(traversal, subdirs, depth, handle, excluded),
        &mut dir_nodes,
        &mut subtree_bytes,
        &mut aggregated_bytes,
        &mut aggregated_entries,
    );
    let size = own_bytes.saturating_add(subtree_bytes);
    let mut children = Vec::with_capacity(dir_nodes.len() + file_nodes.len() + 1);
    children.append(&mut dir_nodes);
    children.append(&mut file_nodes);
    if aggregated_entries > 0 {
        children.push(SizeNode {
            name: crate::analytics::aggregate_name(aggregated_entries).into_boxed_str(),
            size: aggregated_bytes,
            is_dir: false,
            children: Vec::new(),
        });
    }
    SizeNode {
        name,
        size,
        is_dir: true,
        children,
    }
}

fn scan_children(
    traversal: &Traversal<'_>,
    subdirs: Vec<(PathBuf, Box<str>, Retention)>,
    depth: u32,
    handle: Option<&crate::local_access::DirectoryHandle>,
    excluded: &[PathBuf],
) -> Vec<(SizeNode, Retention)> {
    let visit = |(path, name, retention): (PathBuf, Box<str>, Retention)| {
        let child = match handle {
            Some(handle) => match handle.open_child(path.file_name().unwrap_or_default()) {
                Ok(child) => Some(child),
                Err(error) => {
                    traversal.diagnostics.dir_failed(&path, &error, false);
                    return (empty_dir(name), retention);
                }
            },
            None => None,
        };
        (
            scan_dir_with(
                traversal,
                &path,
                name,
                depth.saturating_add(1),
                false,
                child.as_ref(),
                excluded,
            ),
            retention,
        )
    };
    if traversal.progress.cancel.load(Ordering::Relaxed) {
        Vec::new()
    } else if traversal.parallel && subdirs.len() > 1 {
        subdirs.into_par_iter().map(visit).collect()
    } else {
        subdirs.into_iter().map(visit).collect()
    }
}

fn fold_children(
    visited: Vec<(SizeNode, Retention)>,
    dir_nodes: &mut Vec<SizeNode>,
    size: &mut u64,
    aggregated_bytes: &mut u64,
    aggregated_entries: &mut u64,
) {
    for (node, retention) in visited {
        *size = size.saturating_add(node.size);
        match retention {
            Retention::Keep => dir_nodes.push(node),
            Retention::Aggregate => {
                *aggregated_bytes = aggregated_bytes.saturating_add(node.size);
                *aggregated_entries += 1;
            }
        }
    }
}
