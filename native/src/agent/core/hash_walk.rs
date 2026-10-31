//! `hash_walk` over the agent protocol. A server with `walk-meta-v1` (agent
//! or background service of this version) answers `WalkHashed3` in
//! portions and, on request, with the listing attributes of every entry;
//! an older server with `ext-v1` answers `WalkHashed2` one frame per item and
//! never gets a request for attributes (`Ok(false)` without sending).
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Sender;

use super::super::backend::AgentBackend;
use super::super::ext_stream::{is_unsupported_error, stream, unexpected, CANCEL_POLL};
use super::super::ext_wire::{algorithm_to_wire, omission_from_wire};
use crate::agent_proto::{Frame, WireHashEntry};
use crate::vfs::{HashWalkEntry, HashWalkItem, HashWalkMeta, HashWalkRequest, VfsResult};

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
