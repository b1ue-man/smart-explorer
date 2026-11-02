//! The local part of a host hash walk: handle-confined folders read on as
//! many threads as the host offers (`agent_proto::ordered_walk`). Size and
//! time come from the listing of the opened parent; a file is opened only to
//! hash it. The entries and omissions are those of the former serial walk,
//! except that a subfolder is an entry before it is opened: a subfolder that
//! cannot be opened or is refused is that entry followed by an omission of
//! the same path, as the folder walk shows its row and then its failure.
//! The walk's filter (`agent_proto::walk_filter`) leaves a file without its
//! digest (never opened) and a folder unopened; both stay entries.
use super::{child_rel, Batch, Shared, MAX_DEPTH};
use crate::{
    local_access::{DirectoryHandle, EntryKind, LocalEntry},
    share::{
        fs_response::{FsHashEntry, FsHashMeta, FsOmissionReason},
        host_list,
    },
};
use std::{io, path::PathBuf, sync::atomic::Ordering};

/// One folder still to read: its handle, its host path (for the excluded
/// storage roots), its path below the walk root and its depth.
pub(super) type Job = (DirectoryHandle, PathBuf, String, usize);

pub(super) fn walk(
    shared: &Shared<'_>,
    root: DirectoryHandle,
    physical: PathBuf,
    rel: &str,
) -> io::Result<()> {
    let excluded = crate::share::storage_roots::excluded();
    crate::agent_proto::ordered_walk(
        vec![(root, physical, rel.to_string(), 0)],
        crate::agent_proto::walk_threads(),
        shared.cancel,
        || Batch::new(shared),
        |job: Job, local, batch| {
            folder(job, &excluded, local, batch)?;
            batch.end_folder()
        },
    )
}

/// The runs of named folders (`host_hash_walk_runs.rs`): each read as the
/// walk reads it, without going below it.
pub(super) fn walk_folders(shared: &Shared<'_>, jobs: Vec<Job>) -> io::Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }
    let excluded = crate::share::storage_roots::excluded();
    crate::agent_proto::ordered_walk(
        jobs,
        crate::agent_proto::walk_threads(),
        shared.cancel,
        || Batch::new(shared),
        |job: Job, _, batch| {
            let mut detached = crate::agent_proto::WalkLocal::detached();
            folder(job, &excluded, &mut detached, batch)?;
            batch.end_folder()
        },
    )
}

fn folder(
    (directory, physical, rel, depth): Job,
    excluded: &[PathBuf],
    local: &mut crate::agent_proto::WalkLocal<Job>,
    batch: &mut Batch<'_, '_>,
) -> io::Result<()> {
    batch.begin_folder(&rel)?;
    batch.shared().check()?;
    if depth > MAX_DEPTH {
        return batch.hole(
            &rel,
            FsOmissionReason::Unreadable,
            "Verzeichnistiefe überschreitet den sicheren Walk-Stack",
        );
    }
    let entries = match directory.read_directory() {
        Ok(entries) => entries,
        Err(e) => return batch.failed(&rel, e),
    };
    for entry in entries {
        batch.shared().check()?;
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                batch.failed(&rel, error)?;
                continue;
            }
        };
        let name = entry.name.to_string_lossy();
        if host_list::hidden(&name) || excluded.iter().any(|p| p == &physical.join(&entry.name)) {
            continue;
        }
        let child = child_rel(&rel, &name);
        if entry.unreachable
            || entry.name.to_str().is_none()
            || crate::vfs::validate_child_name(&name).is_err()
        {
            batch.hole(
                &child,
                FsOmissionReason::Unrepresentable,
                "Name ist kein adressierbarer Share-Pfad",
            )?;
            continue;
        }
        let link = entry.is_link_like || entry.kind == EntryKind::Link;
        let special = !link && entry.kind == EntryKind::Other;
        let meta = batch.shared().meta(|| attributes(&entry, link, special));
        if link || special {
            if meta.is_some() {
                // As the folder's listing reports it (`VfsMeta`).
                batch.add(Ok(FsHashEntry {
                    rel: child,
                    size: if special { 0 } else { entry.size },
                    mtime_ms: entry.mtime_ms,
                    meta,
                    ..FsHashEntry::default()
                }))?;
            } else if link {
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
        if entry.kind == EntryKind::Directory {
            // The row of the listing first; opening it decides its content.
            let row = FsHashEntry {
                rel: child.clone(),
                is_dir: true,
                // With attributes: the size the folder's listing reports.
                size: if meta.is_some() { entry.size } else { 0 },
                mtime_ms: entry.mtime_ms,
                meta,
                ..FsHashEntry::default()
            };
            if filter.is_some_and(|f| f.leaves_out_dir(&child, entry.hidden)) {
                batch.skipped_dir(row)?;
                continue;
            }
            batch.add(Ok(row))?;
            let opened = directory.open_child(&entry.name).and_then(|handle| {
                crate::share::fs::ensure_local_share_handle_allowed(&handle).map(|()| handle)
            });
            match opened {
                Ok(handle) => local.push((handle, physical.join(&entry.name), child, depth + 1)),
                Err(error) => batch.failed(&child, error)?,
            }
        } else if entry.size >= batch.shared().request.min_bytes {
            let left_out = filter.is_some_and(|f| {
                f.leaves_out_file(&child, entry.hidden, entry.size, entry.mtime_ms)
            });
            let digest = if left_out {
                Ok(None)
            } else {
                file(&directory, &entry, batch.shared())
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
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(error) => batch.failed(&child, error)?,
            }
        }
    }
    Ok(())
}

fn attributes(entry: &LocalEntry, link: bool, special: bool) -> FsHashMeta {
    FsHashMeta {
        hidden: entry.hidden,
        system: entry.system,
        btime_ms: entry.btime_ms,
        is_symlink: link,
        special,
    }
}

/// The requested digest of a listed regular file, read through its parent's
/// handle; without an algorithm the file is not opened at all.
fn file(
    directory: &DirectoryHandle,
    entry: &LocalEntry,
    shared: &Shared<'_>,
) -> io::Result<Option<String>> {
    if shared.request.algo.is_none() {
        return Ok(None);
    }
    let file = directory.open_regular_child(&entry.name)?;
    let before = file.metadata()?;
    if before.len() != entry.size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Datei nach der Auflistung geändert",
        ));
    }
    // The host's digest cache (`agent_proto::hash_cache`): an unchanged file
    // is not read again, unless the request reads fresh (the checksum compare
    // mode): then the file is read and the digest read is stored.
    let code = match shared.request.algo {
        Some(crate::share::wire::FsHashAlgo::Md5) => crate::agent_proto::digest::MD5,
        Some(crate::share::wire::FsHashAlgo::Sha256) => crate::agent_proto::digest::SHA256,
        _ => return shared.digest(&file, entry.size),
    };
    let read = || {
        shared
            .digest(&file, entry.size)?
            .ok_or_else(|| io::Error::other("Prüfsumme fehlt"))
    };
    let stamp = || crate::agent_proto::file_stamp(&file);
    let digest = if shared.fresh {
        crate::agent_proto::hash_cache::digest_fresh(stamp(), code, read, stamp)
    } else {
        crate::agent_proto::hash_cache::digest(stamp(), code, read, stamp)
    }
    .map(Some)?;
    let after = file.metadata()?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Datei beim Hashen geändert",
        ));
    }
    Ok(digest)
}
