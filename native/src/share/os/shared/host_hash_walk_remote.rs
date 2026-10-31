//! The part of a host hash walk below an exported connection (a remote
//! backend the host reaches itself): one listing after the other, as its
//! own connection bounds the pace. A digest is read over that connection, so
//! a file the walk's filter leaves out (`agent_proto::walk_filter`) is never
//! read, and a folder it leaves out is never listed: both still come as
//! entries.
use super::{child_rel, Batch, MAX_DEPTH};
use crate::share::{
    fs_response::{FsHashEntry, FsHashMeta, FsOmissionReason},
    host_list,
};
use std::{io, sync::atomic::Ordering};

/// An exported connection's folder, one listing after the other: the
/// folder's items, then its subfolders.
pub(super) fn walk(
    batch: &mut Batch<'_, '_>,
    backend: &dyn crate::vfs::Backend,
    path: &str,
    rel: &str,
    depth: usize,
) -> io::Result<()> {
    batch.begin_folder(rel)?;
    let subfolders = folder(batch, backend, path, rel, depth)?;
    batch.end_folder()?;
    for (full, child) in subfolders {
        walk(batch, backend, &full, &child, depth + 1)?;
    }
    Ok(())
}

/// The items of one folder of an exported connection; returns its
/// subfolders (path, rel) to walk next.
pub(super) fn folder(
    batch: &mut Batch<'_, '_>,
    backend: &dyn crate::vfs::Backend,
    path: &str,
    rel: &str,
    depth: usize,
) -> io::Result<Vec<(String, String)>> {
    let mut subfolders = Vec::new();
    batch.shared().check()?;
    if depth > MAX_DEPTH {
        batch.hole(
            rel,
            FsOmissionReason::Unreadable,
            "Verzeichnistiefe überschreitet den sicheren Walk-Stack",
        )?;
        return Ok(subfolders);
    }
    let listing = match crate::vfs::list_dir_tolerant(backend, path) {
        Ok(listing) => listing,
        Err(error) => {
            batch.failed(rel, error)?;
            return Ok(subfolders);
        }
    };
    for hole in listing.omitted {
        batch.hole(&child_rel(rel, &hole.rel), hole.reason.into(), hole.detail)?;
    }
    for entry in listing.entries {
        batch.shared().check()?;
        if host_list::hidden(&entry.name) {
            continue;
        }
        let child = child_rel(rel, &entry.name);
        if crate::vfs::validate_child_name(&entry.name).is_err() {
            batch.hole(
                &child,
                FsOmissionReason::Unrepresentable,
                "Name ist kein Share-Pfad",
            )?;
            continue;
        }
        let meta = batch.shared().meta(|| FsHashMeta {
            hidden: entry.hidden,
            system: entry.system,
            btime_ms: entry.btime_ms,
            is_symlink: entry.is_symlink,
            special: entry.special && !entry.is_symlink,
        });
        if entry.is_symlink || entry.special {
            if meta.is_some() {
                batch.add(Ok(FsHashEntry {
                    rel: child,
                    size: if entry.special { 0 } else { entry.size },
                    mtime_ms: entry.mtime_ms,
                    meta,
                    ..FsHashEntry::default()
                }))?;
            } else if entry.is_symlink {
                batch.hole(&child, FsOmissionReason::Link, "Link-Grenze")?;
            } else {
                batch.hole(
                    &child,
                    FsOmissionReason::Special,
                    "Kein regulärer Datenstrom",
                )?;
            }
            continue;
        }
        let filter = batch.shared().filter();
        if entry.is_dir && filter.is_some_and(|f| f.leaves_out_dir(&child, entry.hidden)) {
            batch.skipped_dir(FsHashEntry {
                rel: child,
                is_dir: true,
                size: if meta.is_some() { entry.size } else { 0 },
                mtime_ms: entry.mtime_ms,
                meta,
                ..FsHashEntry::default()
            })?;
            continue;
        }
        let full = match crate::vfs::sync_child_path(backend, path, &entry.name) {
            Ok(full) => full,
            Err(error) => {
                batch.failed(&child, error)?;
                continue;
            }
        };
        if entry.is_dir {
            batch.add(Ok(FsHashEntry {
                rel: child.clone(),
                is_dir: true,
                size: if meta.is_some() { entry.size } else { 0 },
                mtime_ms: entry.mtime_ms,
                meta,
                ..FsHashEntry::default()
            }))?;
            subfolders.push((full, child));
        } else if entry.size >= batch.shared().request.min_bytes {
            let left_out = filter.is_some_and(|f| {
                f.leaves_out_file(&child, entry.hidden, entry.size, entry.mtime_ms)
            });
            let digest = if batch.shared().request.algo.is_none() || left_out {
                Ok(None)
            } else {
                crate::vfs::open_read_regular(backend, &full, entry.id.as_deref())
                    .and_then(|read| batch.shared().digest(read, entry.size))
            };
            match digest {
                Ok(digest) => {
                    batch
                        .shared()
                        .progress
                        .files
                        .fetch_add(1, Ordering::Relaxed);
                    batch.add(Ok(FsHashEntry {
                        rel: child,
                        size: entry.size,
                        mtime_ms: entry.mtime_ms,
                        digest,
                        meta,
                        ..FsHashEntry::default()
                    }))?;
                }
                Err(e) => batch.failed(&child, e)?,
            }
        }
    }
    Ok(subfolders)
}
