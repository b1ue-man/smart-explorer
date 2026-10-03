//! Bounded server-side walks, preserving directories, filters and omissions.
use super::omissions::{OmissionKind, SyncOmissions};
use super::snapshot::{Snapshot, WalkFilter};
use super::snapshot_hash::{md5_hex_to_u64, HashMode};
use super::snapshot_types::DirSet;
use super::types::{BisyncOptions, Sig, Tree};
use crate::vfs::{Backend, HashWalkItem, HashWalkRequest};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn walk_snapshot_via_agent(
    backend: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
    opts: BisyncOptions,
    fold_case: bool,
) -> io::Result<Option<Snapshot>> {
    // The old protocol has no cross-mount policy field. Metadata walking is
    // authoritative when the caller requests that boundary; local walks
    // also need the device's own-data and previously mounted-path policy.
    if backend.is_local()
        || !opts.cross_mounts
        || !filter.include_hidden
        || backend.has_duplicate_file_names()
    {
        return Ok(None);
    }
    super::transfer_stream::check(cancel)?;
    let want_hash = matches!(hash, HashMode::Full | HashMode::FullFresh);
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let mut out = Snapshot {
        tree: Tree::new(),
        filtered: Tree::new(),
        dirs: DirSet::new(),
        omissions: SyncOmissions::new(fold_case),
        duplicates: Default::default(),
    };
    let nodes = std::cell::Cell::new(0u64);
    let text = std::cell::Cell::new(0u64);
    let mut failure = None;
    let consume = &mut |item: HashWalkItem| {
        if cancel.load(Ordering::Acquire) {
            failure.get_or_insert_with(canceled);
            return;
        }
        if failure.is_some() {
            return;
        }
        let rel = match &item {
            HashWalkItem::Entry(entry) => &entry.rel,
            HashWalkItem::Omitted(omission) => &omission.rel,
        };
        if rel.is_empty() {
            return;
        }
        nodes.set(nodes.get().saturating_add(1));
        text.set(text.get().saturating_add(rel.len() as u64));
        if nodes.get() > limits.walk_entries || text.get() > limits.walk_text_bytes {
            failure = Some(invalid("agent walk exceeds its collection budget"));
            return;
        }
        if let Err(error) = crate::agent_proto::ValidatedRelativePath::parse(rel) {
            failure = Some(error);
            return;
        }
        let own = rel.split('/').any(|name| {
            super::paths::is_engine_name(name)
                || crate::vfs::is_staging_name(name)
                || crate::apptrash::excluded_name(name)
        });
        let filtered_parent = rel
            .match_indices('/')
            .any(|(end, _)| filter.ignored(&rel[..end], true))
            || (!filter.include_hidden && rel.split('/').any(|name| name.starts_with('.')));
        if own {
            out.omissions.record_kind(rel, OmissionKind::OwnFile, false);
            return;
        }
        match item {
            HashWalkItem::Omitted(omission) => {
                out.omissions.record_kind(
                    &omission.rel,
                    omission.reason.into(),
                    !filtered_parent && !filter.ignored(&omission.rel, true),
                );
            }
            HashWalkItem::Entry(entry) => {
                let filtered = filtered_parent
                    || filter.ignored(&entry.rel, entry.is_dir)
                    || (!entry.is_dir && !filter.size_age_ok(entry.size, entry.mtime_ms));
                if entry.is_dir {
                    if filtered {
                        out.omissions
                            .record_kind(&entry.rel, OmissionKind::Filtered, false);
                    } else {
                        out.dirs.insert(entry.rel);
                    }
                } else {
                    let checksum = entry.digest.as_deref().map(md5_hex_to_u64).unwrap_or(0);
                    if want_hash && checksum == 0 {
                        failure = Some(invalid("agent returned a missing or invalid digest"));
                        return;
                    }
                    let signature = Sig {
                        size: entry.size,
                        mtime_ms: entry.mtime_ms,
                        hash: if hash == HashMode::None { 0 } else { checksum },
                    };
                    let tree = if filtered {
                        &mut out.filtered
                    } else {
                        &mut out.tree
                    };
                    if tree.insert(entry.rel, signature).is_some() {
                        failure = Some(invalid("agent returned duplicate sync paths"));
                    }
                }
            }
        }
    };
    let (sender, receiver) = crossbeam_channel::bounded(1024);
    let primary = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            crate::vfs::hash_walk(
                backend,
                root,
                HashWalkRequest {
                    algorithm: want_hash.then_some(crate::analytics::HashAlgorithm::Md5),
                    min_bytes: 0,
                },
                sender,
                cancel,
            )
        });
        for item in receiver.iter() {
            consume(item);
        }
        worker.join()
    });
    let primary = match primary {
        Ok(Ok(ran)) => ran,
        Ok(Err(error)) if error.to_string() == crate::agent_proto::HASH_WALK_LINK_BOUNDARY => {
            return Ok(None)
        }
        Ok(Err(error)) => return Err(error),
        Err(_) => return Err(io::Error::other("hash walk worker panicked")),
    };
    if !primary {
        if nodes.get() != 0 {
            return Err(invalid("hash walk refused after emitting entries"));
        }
        let (sender, receiver) = crossbeam_channel::bounded::<crate::vfs::HashHit>(1024);
        let legacy = std::thread::scope(|scope| {
            let worker = scope.spawn(|| backend.walk_hashed(root, want_hash, sender, cancel));
            for hit in receiver.iter() {
                consume(HashWalkItem::Entry(crate::vfs::HashWalkEntry {
                    rel: hit.rel,
                    is_dir: hit.is_dir,
                    size: hit.size,
                    mtime_ms: hit.mtime_ms,
                    digest: hit.md5,
                }));
            }
            worker.join()
        });
        match legacy {
            Ok(Ok(false)) if nodes.get() == 0 => return Ok(None),
            Ok(Ok(false)) => {
                return Err(invalid("legacy hash walk refused after emitting entries"))
            }
            Ok(Ok(true)) => {}
            Ok(Err(error)) if error.to_string() == crate::agent_proto::HASH_WALK_LINK_BOUNDARY => {
                return Ok(None)
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(io::Error::other("legacy hash walk worker panicked")),
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    super::transfer_stream::check(cancel)?;
    out.omissions.exclude_tree(&mut out.tree);
    out.dirs.retain(|rel| !out.omissions.contains(rel));
    Ok(Some(out))
}

pub(super) fn walk_hashed_via_agent(
    backend: &dyn Backend,
    root: &str,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    hash: HashMode,
) -> io::Result<Option<Tree>> {
    walk_snapshot_via_agent(
        backend,
        root,
        cancel,
        filter,
        hash,
        BisyncOptions {
            cross_mounts: true,
            ..Default::default()
        },
        false,
    )
    .map(|snapshot| snapshot.map(|snapshot| snapshot.tree))
}
fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "agent sync walk canceled")
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
