//! `hash_walk` over the agent protocol. A server with `walk-meta-v1` (agent
//! or background service of this version) answers `WalkHashed3` in
//! portions and, on request, with the listing attributes of every entry;
//! an older server with `ext-v1` answers `WalkHashed2` one frame per item and
//! never gets a request for attributes (`Ok(false)` without sending). A
//! server with `walk-filter-v1` gets the request's scope (fresh digests and
//! the job's filter, `agent_proto::walk_filter`); any other server walks
//! without it, which the caller's own filter covers.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Sender;

use super::super::backend::AgentBackend;
use super::super::ext_stream::{is_unsupported_error, stream, unexpected, CANCEL_POLL};
use super::super::ext_wire::{algorithm_to_wire, omission_from_wire};
use crate::agent_proto::{Frame, ServerFeatures, WalkScope, WireHashEntry};
use crate::vfs::{
    HashWalkEntry, HashWalkItem, HashWalkMeta, HashWalkRequest, KeyedWalk, KeyedWalkItem, VfsResult,
};

pub(super) fn walk(
    backend: &AgentBackend,
    root: &str,
    request: HashWalkRequest,
    tx: Sender<HashWalkItem>,
    cancel: &AtomicBool,
) -> VfsResult<bool> {
    let features = backend.features();
    if !features.extensions || (request.meta && !features.walk_meta) {
        return Ok(false);
    }
    let parts = features.walk_meta;
    let algorithm = algorithm_to_wire(request.algorithm);
    let frame = if parts {
        Frame::WalkHashed3 {
            root: root.to_string(),
            algorithm,
            min_bytes: request.min_bytes,
            meta: request.meta,
            scope: scope_of(&request, &features),
        }
    } else {
        Frame::WalkHashed2 {
            root: root.to_string(),
            algorithm,
            min_bytes: request.min_bytes,
        }
    };
    let mut items = 0u64;
    let receiver_gone = || {
        if cancel.load(Ordering::Relaxed) {
            io::Error::new(io::ErrorKind::Interrupted, "agent hash walk canceled")
        } else {
            io::Error::new(io::ErrorKind::BrokenPipe, "hash-walk receiver closed")
        }
    };
    let mut forward = |item: HashWalkItem| -> io::Result<()> {
        items += 1;
        // A full downstream queue must not hide a cancel from the host.
        let mut pending = item;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(receiver_gone());
            }
            match tx.send_timeout(pending, CANCEL_POLL) {
                Ok(()) => return Ok(()),
                Err(crossbeam_channel::SendTimeoutError::Timeout(item)) => pending = item,
                Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                    return Err(receiver_gone())
                }
            }
        }
    };
    let result = stream(&backend.pool, frame, cancel, |frame| match frame {
        Frame::HashEntry {
            rel,
            is_dir,
            size,
            mtime_ms,
            md5,
        } if !parts => forward(HashWalkItem::Entry(HashWalkEntry {
            rel,
            is_dir,
            size,
            mtime_ms,
            digest: md5,
            meta: None,
        })),
        Frame::HashOmitted(omitted) if !parts => {
            forward(HashWalkItem::Omitted(omission_from_wire(omitted)))
        }
        Frame::HashPart { entries, omitted } if parts => {
            for entry in entries {
                forward(HashWalkItem::Entry(entry_from_wire(entry, request.meta)?))?;
            }
            for omitted in omitted {
                forward(HashWalkItem::Omitted(omission_from_wire(omitted)))?;
            }
            Ok(())
        }
        other => Err(unexpected("hash walk", &other)),
    });
    match result {
        Ok(()) => Ok(true),
        Err(error) if items == 0 && is_unsupported_error(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

/// `WalkKeyed` (`walk-keys-v1`): the walk with listing attributes whose
/// folders carry keys; `Ok(false)` without the label, before sending.
pub(super) fn walk_keyed(
    backend: &AgentBackend,
    root: &str,
    request: HashWalkRequest,
    walk: KeyedWalk<'_>,
    tx: Sender<KeyedWalkItem>,
    cancel: &AtomicBool,
) -> VfsResult<bool> {
    let features = backend.features();
    if !features.extensions || !features.walk_keys || !request.meta {
        return Ok(false);
    }
    let algorithm = algorithm_to_wire(request.algorithm);
    let scope = scope_of(&request, &features);
    let frames = match walk {
        KeyedWalk::Marks { known } => vec![Frame::WalkKeyed {
            root: root.to_string(),
            algorithm,
            min_bytes: request.min_bytes,
            // The client puts the keys covering most first.
            known: known[..known.len().min(crate::agent_proto::MAX_KNOWN_KEYS)].concat(),
            keys_only: false,
            scope,
        }],
        KeyedWalk::KeysOnly => vec![Frame::WalkKeyed {
            root: root.to_string(),
            algorithm,
            min_bytes: request.min_bytes,
            known: Vec::new(),
            keys_only: true,
            scope,
        }],
        // Portions of at most `RUN_FOLDERS_BYTES` path text each.
        KeyedWalk::Runs { folders } => run_portions(folders)
            .into_iter()
            .map(|folders| Frame::ListRuns {
                root: root.to_string(),
                algorithm,
                folders,
                scope: scope.clone(),
            })
            .collect(),
    };
    let mut items = 0u64;
    let mut forward = |item: KeyedWalkItem| -> io::Result<()> {
        items += 1;
        let mut pending = item;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "agent hash walk canceled",
                ));
            }
            match tx.send_timeout(pending, CANCEL_POLL) {
                Ok(()) => return Ok(()),
                Err(crossbeam_channel::SendTimeoutError::Timeout(item)) => pending = item,
                Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "hash-walk receiver closed",
                    ))
                }
            }
        }
    };
    for frame in frames {
        let result = stream(&backend.pool, frame, cancel, |frame| match frame {
            Frame::HashPart { entries, omitted } => {
                for entry in entries {
                    let entry = entry_from_wire(entry, true)?;
                    forward(KeyedWalkItem::Item(HashWalkItem::Entry(entry)))?;
                }
                for omitted in omitted {
                    let omitted = omission_from_wire(omitted);
                    forward(KeyedWalkItem::Item(HashWalkItem::Omitted(omitted)))?;
                }
                Ok(())
            }
            Frame::FolderRun(rel) => forward(KeyedWalkItem::Run { rel }),
            Frame::FolderKey {
                rel,
                key,
                same,
                tally,
            } => forward(KeyedWalkItem::Key {
                rel,
                key,
                same,
                tally,
            }),
            other => Err(unexpected("keyed hash walk", &other)),
        });
        match result {
            Ok(()) => {}
            Err(error) if items == 0 && is_unsupported_error(&error) => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

/// The scope a server with `walk-filter-v1` gets; the filter only with
/// listing attributes (the `hidden` it applies is theirs).
fn scope_of(request: &HashWalkRequest, features: &ServerFeatures) -> WalkScope {
    if !features.walk_filter {
        return WalkScope::default();
    }
    WalkScope {
        fresh: request.fresh,
        filter: request.filter.as_deref().filter(|_| request.meta).cloned(),
    }
}

/// Path text one `ListRuns` carries at most.
const RUN_FOLDERS_BYTES: usize = 8 * 1024 * 1024;

/// `folders` in portions of at most `RUN_FOLDERS_BYTES` path text.
fn run_portions(folders: &[String]) -> Vec<Vec<String>> {
    let mut portions: Vec<Vec<String>> = Vec::new();
    let mut bytes = usize::MAX;
    for folder in folders {
        if bytes.saturating_add(folder.len() + 4) > RUN_FOLDERS_BYTES {
            portions.push(Vec::new());
            bytes = 0;
        }
        bytes += folder.len() + 4;
        if let Some(portion) = portions.last_mut() {
            portion.push(folder.clone());
        }
    }
    portions
}

/// An entry of a `HashPart`; its attributes must be there exactly when the
/// walk asked for them.
fn entry_from_wire(entry: WireHashEntry, meta: bool) -> io::Result<HashWalkEntry> {
    if entry.meta.is_some() != meta {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "hash walk entry attributes do not match the request",
        ));
    }
    Ok(HashWalkEntry {
        rel: entry.rel,
        is_dir: entry.is_dir,
        size: entry.size,
        mtime_ms: entry.mtime_ms,
        digest: entry.digest,
        meta: entry.meta.map(|meta| HashWalkMeta {
            hidden: meta.hidden,
            system: meta.system,
            btime_ms: meta.btime_ms,
            is_symlink: meta.is_symlink,
            special: meta.special,
        }),
    })
}
