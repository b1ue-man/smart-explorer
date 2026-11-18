//! Directory assembly of the size scanner; handles confine peer scans.
use super::assembly::Assembly;
use super::*;
use std::ffi::OsStr;

/// The occupied bytes this walk counts for a file: its allocation, 0 for a
/// further link of a hard-link group already counted in this analysis, and
/// its size (noted as unknown) where the platform could not say.
fn occupied(traversal: &Traversal<'_>, dir: &Path, entry: &LocalEntry) -> u64 {
    let Some(allocated) = entry.allocated else {
        traversal.diagnostics.allocation_unknown();
        return entry.size;
    };
    match entry.hardlink {
        Some(identity) if !traversal.first_link(identity, dir) => 0,
        _ => allocated,
    }
}

/// A folder on another file system than the scan root is left out (like
/// `du -x`, and like a Windows mount point, which is a link): noted, not
/// counted. Unknown devices never exclude.
pub(super) fn foreign_device(traversal: &Traversal<'_>, path: &Path, device: Option<u64>) -> bool {
    let foreign = matches!((traversal.device, device), (Some(root), Some(child)) if root != child);
    if foreign {
        traversal.diagnostics.note(format!(
            "anderes Dateisystem, nicht gezählt: {}",
            crate::analytics::os::display_path(path)
        ));
    }
    foreign
}

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
    let mut assembly = Assembly::default();
    // The excluded children of this folder, compared by name only.
    let hidden: Vec<&OsStr> = excluded
        .iter()
        .filter(|path| path.parent() == Some(dir))
        .filter_map(|path| path.file_name())
        .collect();

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
                // Borrowed for valid UTF-8; copied only where it is kept.
                let nm = ent.name.to_string_lossy();
                // The app trash (Android) is left out like in every other walk.
                if crate::apptrash::excluded_name(&nm)
                    || crate::vfs::is_engine_name(&nm)
                    || crate::vfs::is_staging_name(&nm)
                    || hidden.iter().any(|hidden| *hidden == ent.name.as_os_str())
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
                    if foreign_device(traversal, &path, ent.device) {
                        continue;
                    }
                    let retention =
                        budget.claim(&path, depth.saturating_add(1), nm.len() as u64, diagnostics);
                    subdirs.push((path, nm.into_owned().into_boxed_str(), retention));
                    if subdirs.len() == 256 {
                        p.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);
                        for (node, retention) in scan_children(
                            traversal,
                            std::mem::take(&mut subdirs),
                            depth,
                            handle,
                            excluded,
                        ) {
                            assembly.child(node, retention);
                        }
                        if crate::analytics::Progress::directory_due() {
                            p.enter_directory(&crate::analytics::os::display_path(dir));
                        }
                    }
                } else if ent.kind == EntryKind::File {
                    let allocated = occupied(traversal, dir, &ent);
                    assembly.file(&nm, ent.size, allocated);
                    assembly.report(p, false);
                }
            }
        }
        Err(error) => diagnostics.dir_failed(dir, &error, is_root),
    }

    assembly.report(p, true);
    p.dirs.fetch_add(subdirs.len() as u64, Ordering::Relaxed);
    for (node, retention) in scan_children(traversal, subdirs, depth, handle, excluded) {
        assembly.child(node, retention);
    }
    assembly.finish(name, dir, depth, budget, diagnostics)
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
                // A size listing names folders without a stat: the opened
                // folder tells its file system.
                Ok(child) if foreign_device(traversal, &path, child.device_id()) => return None,
                Ok(child) => Some(child),
                Err(error) => {
                    traversal.diagnostics.dir_failed(&path, &error, false);
                    return Some((empty_dir(name), retention));
                }
            },
            None => None,
        };
        Some((
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
        ))
    };
    if traversal.progress.cancel.load(Ordering::Relaxed) {
        Vec::new()
    } else if traversal.parallel && subdirs.len() > 1 {
        subdirs.into_par_iter().filter_map(visit).collect()
    } else {
        subdirs.into_iter().filter_map(visit).collect()
    }
}
