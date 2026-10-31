//! `WalkHashed2` (`ext-v1`) and `WalkHashed3` (`walk-meta-v1`, in portions
//! and on request with listing attributes) for the backend the service
//! serves, and its tolerant listing with attributes (`ListMeta`). A backend that walks next to its data (a Share host with
//! `hash_walk_v1`, and `hash_walk_meta_v1` for attributes) streams its own
//! items; every other one is walked here: tolerant listings, links, special
//! files and unreadable entries as omissions (with attributes: links and
//! special files as entries), never a failed walk; files below the minimum
//! size left out, digests read only where asked.
//! A SHA-256 walk on one regular file is the file digest of
//! `query::FILE_DIGEST` (`backend_file_digest.rs`): hashed where it is
//! stored, never read here.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::agent::ext_wire::{algorithm_from_wire, omission_to_wire};
use crate::agent::vfs_to_hash_entry;
use crate::agent_proto::WireOmission;
use crate::agent_proto::{emit_meta_listing_parts, Frame, WireHashEntry, WireHashMeta};
use crate::vfs::{self as vfs, BackendHandle, HashWalkItem, HashWalkRequest};

use super::backend_budget::WalkBudget;
use super::backend_server::{emit, Sink};

#[cfg(all(test, unix))]
#[path = "backend_abgleich_task_tests.rs"]
mod backend_abgleich_task_tests;
#[cfg(test)]
#[path = "backend_file_digest_task_tests.rs"]
mod backend_file_digest_task_tests;
#[path = "backend_file_digest.rs"]
mod file_digest;
#[path = "backend_hash_keyed.rs"]
mod keyed;
#[path = "backend_hash_walk.rs"]
mod local_walk;
use local_walk::Walk;

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "daemon hash walk canceled")
}

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", parent.trim_end_matches('/'))
    }
}

fn rel_join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Estimated encoded bytes after which a `HashPart` is sent.
const PART_BYTES: usize = 256 * 1024;

/// Sends the items of one walk: one frame each (`WalkHashed2`) or in
/// `HashPart`s (`WalkHashed3`) that keep the order of the relayed walk: the
/// client forwards a part's entries before its omissions, so a part never
/// holds an entry after an omission (the folder runs of a `meta` walk reach
/// the client as the storing host sent them).
struct Items<'a> {
    sink: &'a Sink,
    id: u64,
    parts: bool,
    entries: Vec<WireHashEntry>,
    omitted: Vec<WireOmission>,
    bytes: usize,
}

impl<'a> Items<'a> {
    fn new(sink: &'a Sink, id: u64, parts: bool) -> Self {
        Self {
            sink,
            id,
            parts,
            entries: Vec::new(),
            omitted: Vec::new(),
            bytes: 0,
        }
    }

    fn send(&mut self, item: HashWalkItem) -> io::Result<()> {
        if !self.parts {
            let frame = match item {
                HashWalkItem::Entry(entry) => Frame::HashEntry {
                    rel: entry.rel,
                    is_dir: entry.is_dir,
                    size: entry.size,
                    mtime_ms: entry.mtime_ms,
                    md5: entry.digest,
                },
                HashWalkItem::Omitted(omitted) => Frame::HashOmitted(omission_to_wire(omitted)),
            };
            return emit(self.sink, self.id, &frame);
        }
        if matches!(item, HashWalkItem::Entry(_)) && !self.omitted.is_empty() {
            self.flush()?;
        }
        match item {
            HashWalkItem::Entry(entry) => {
                self.bytes += entry.rel.len() + entry.digest.as_ref().map_or(0, String::len) + 40;
                self.entries.push(WireHashEntry {
                    rel: entry.rel,
                    is_dir: entry.is_dir,
                    size: entry.size,
                    mtime_ms: entry.mtime_ms,
                    digest: entry.digest,
                    meta: entry.meta.map(|meta| WireHashMeta {
                        hidden: meta.hidden,
                        system: meta.system,
                        btime_ms: meta.btime_ms,
                        is_symlink: meta.is_symlink,
                        special: meta.special,
                    }),
                });
            }
            HashWalkItem::Omitted(omitted) => {
                let omitted = omission_to_wire(omitted);
                self.bytes += omitted.rel.len() + omitted.detail.len() + 16;
                self.omitted.push(omitted);
            }
        }
        if self.bytes >= PART_BYTES {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.bytes = 0;
        if self.entries.is_empty() && self.omitted.is_empty() {
            return Ok(());
        }
        emit(
            self.sink,
            self.id,
            &Frame::HashPart {
                entries: std::mem::take(&mut self.entries),
                omitted: std::mem::take(&mut self.omitted),
            },
        )
    }
}

/// Whether `serve` answers `frame`.
pub(super) fn serves(frame: &Frame) -> bool {
    matches!(
        frame,
        Frame::WalkHashed2 { .. }
            | Frame::WalkHashed3 { .. }
            | Frame::WalkKeyed { .. }
            | Frame::ListRuns { .. }
            | Frame::ListMeta(_)
            | Frame::ListDirMeta(_)
            | Frame::StatMeta(_)
    )
}

/// `WalkHashed2`, `WalkHashed3`, `WalkKeyed`, `ListMeta`, `ListDirMeta` or
/// `StatMeta` of the served backend.
pub(super) fn serve(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    request: Frame,
    cancel: &AtomicBool,
) -> io::Result<()> {
    match request {
        Frame::WalkHashed2 {
            root,
            algorithm,
            min_bytes,
        } => walk_hashed2(sink, id, backend, &root, algorithm, min_bytes, cancel),
        Frame::WalkHashed3 {
            root,
            algorithm,
            min_bytes,
            meta,
            scope,
        } => walk_hashed3(
            sink,
            id,
            backend,
            &root,
            (algorithm, min_bytes, meta, scope),
            cancel,
        ),
        Frame::WalkKeyed {
            root,
            algorithm,
            min_bytes,
            known,
            keys_only,
            scope,
        } => {
            check_algorithm(algorithm)?;
            let relayed = keyed::Relayed::Walk {
                min_bytes,
                known: &known,
                keys_only,
            };
            let walk = (root.as_str(), algorithm, scope);
            keyed::serve(sink, id, backend, walk, relayed, cancel)
        }
        Frame::ListRuns {
            root,
            algorithm,
            folders,
            scope,
        } => {
            check_algorithm(algorithm)?;
            let relayed = keyed::Relayed::Runs(&folders);
            let walk = (root.as_str(), algorithm, scope);
            keyed::serve(sink, id, backend, walk, relayed, cancel)
        }
        // `list_dir` and `stat` as the served backend reports them, hidden
        // and system flags and creation times included (`meta-list-v1`).
        Frame::ListDirMeta(path) => {
            let entries = vfs::Backend::list_dir(&**backend, &path)?
                .into_iter()
                .map(vfs_to_hash_entry);
            emit_meta_listing_parts(entries, Vec::new(), |part| emit(sink, id, &part))?;
            emit(sink, id, &Frame::End)
        }
        Frame::StatMeta(path) => {
            let entry = vfs_to_hash_entry(vfs::Backend::stat(&**backend, &path)?);
            emit_meta_listing_parts([entry], Vec::new(), |part| emit(sink, id, &part))?;
            emit(sink, id, &Frame::End)
        }
        // The listing as the served backend reports it, hidden and system
        // flags and creation times included (`list-meta-v1`).
        Frame::ListMeta(path) => {
            let listing = vfs::list_dir_tolerant(&**backend, &path)?;
            let entries = listing.entries.into_iter().map(vfs_to_hash_entry);
            let omitted = listing.omitted.into_iter().map(omission_to_wire);
            emit_meta_listing_parts(entries, omitted, |part| emit(sink, id, &part))?;
            emit(sink, id, &Frame::End)
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no hash walk request: {other:?}"),
        )),
    }
}

fn walk_hashed2(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    algorithm: u8,
    min_bytes: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    check_algorithm(algorithm)?;
    if algorithm == crate::agent_proto::digest::SHA256
        && file_digest::serve(sink, id, backend, root, cancel)?
    {
        return Ok(());
    }
    let request = HashWalkRequest {
        algorithm: algorithm_from_wire(algorithm),
        min_bytes,
        meta: false,
        fresh: false,
        filter: None,
    };
    walk(
        &mut Items::new(sink, id, false),
        backend,
        root,
        request,
        cancel,
    )
}

/// `WalkHashed3`: the backend's own walk (without the one-file digest) in
/// portions; `meta` asks for listing attributes, `scope` for fresh digests
/// and the job's filter (`walk-filter-v1`), passed on to the backend.
fn walk_hashed3(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    (algorithm, min_bytes, meta, scope): (u8, u64, bool, crate::agent_proto::WalkScope),
    cancel: &AtomicBool,
) -> io::Result<()> {
    check_algorithm(algorithm)?;
    let request = HashWalkRequest {
        algorithm: algorithm_from_wire(algorithm),
        min_bytes,
        meta,
        fresh: scope.fresh,
        filter: scope.filter.map(std::sync::Arc::new),
    };
    walk(
        &mut Items::new(sink, id, true),
        backend,
        root,
        request,
        cancel,
    )
}

fn check_algorithm(algorithm: u8) -> io::Result<()> {
    if !matches!(
        algorithm,
        crate::agent_proto::digest::NONE
            | crate::agent_proto::digest::MD5
            | crate::agent_proto::digest::SHA256
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unbekannter Hashalgorithmus",
        ));
    }
    Ok(())
}

/// The backend's own walk where it has one, then `End`. Without one a
/// `WalkHashed2` is walked here; a `WalkHashed3` is answered as unsupported,
/// so the client lists the folders itself, in parallel, through this
/// service (faster than one listing after the other here, same result).
fn walk(
    items: &mut Items<'_>,
    backend: &BackendHandle,
    root: &str,
    request: HashWalkRequest,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if vfs::supports_hash_walk(&**backend, root)?
        && relay(items, backend, root, request.clone(), cancel)?
    {
        items.flush()?;
        return emit(items.sink, items.id, &Frame::End);
    }
    if items.parts {
        return emit(
            items.sink,
            items.id,
            &Frame::Err(crate::agent_proto::UNSUPPORTED_EXTENSION.to_string()),
        );
    }
    let mut walk = Walk {
        items,
        backend,
        request,
        cancel,
        budget: WalkBudget::streaming(),
    };
    walk.run(root)?;
    walk.items.flush()?;
    emit(walk.items.sink, walk.items.id, &Frame::End)
}
/// Streams the backend's own walk; `false` when it turned out unsupported
/// before its first item (then this side walks).
fn relay(
    items: &mut Items<'_>,
    backend: &BackendHandle,
    root: &str,
    request: HashWalkRequest,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    let (tx, rx) = crossbeam_channel::bounded::<HashWalkItem>(1024);
    let stop = AtomicBool::new(false);
    let mut forwarded = Ok(());
    let walked = std::thread::scope(|scope| {
        let walker = scope.spawn(|| vfs::hash_walk(&**backend, root, request, tx, &stop));
        loop {
            if cancel.load(Ordering::Relaxed) {
                stop.store(true, Ordering::Relaxed);
            }
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(item) if forwarded.is_ok() => {
                    forwarded = items.send(item);
                    if forwarded.is_err() {
                        stop.store(true, Ordering::Relaxed);
                    }
                }
                Ok(_) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
        walker.join()
    });
    forwarded?;
    match walked {
        Ok(Ok(walked)) => Ok(walked),
        Ok(Err(_)) if cancel.load(Ordering::Relaxed) => Err(canceled()),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(io::Error::other("hash walk of the peer stopped")),
    }
}
