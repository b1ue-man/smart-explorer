//! `WalkHashed2` (`ext-v1`) and `WalkHashed3` (`walk-meta-v1`, in portions
//! and on request with listing attributes) for the backend the service
//! serves. A backend that walks next to its data (a Share host with
//! `hash_walk_v1`, and `hash_walk_meta_v1` for attributes) streams its own
//! items; every other one is walked here: tolerant listings, links, special
//! files and unreadable entries as omissions (with attributes: links and
//! special files as entries), never a failed walk; files below the minimum
//! size left out, digests read only where asked.
//! A SHA-256 walk on one regular file is the file digest of
//! `query::FILE_DIGEST` (`backend_file_digest.rs`): hashed where it is
//! stored, never read here.
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use ring::digest::{Context, SHA256};

use crate::agent::ext_wire::{algorithm_from_wire, omission_to_wire};
use crate::agent_proto::{Frame, WireHashEntry, WireHashMeta, WireOmission, CHUNK};
use crate::analytics::HashAlgorithm;
use crate::vfs::{
    self as vfs, BackendHandle, HashWalkItem, HashWalkRequest, OmissionReason, VfsOmission,
};

use super::backend_budget::WalkBudget;
use super::backend_server::{emit, Sink};

#[cfg(test)]
#[path = "backend_file_digest_task_tests.rs"]
mod backend_file_digest_task_tests;
#[path = "backend_file_digest.rs"]
mod file_digest;

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
/// `HashPart`s (`WalkHashed3`).
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

/// `WalkHashed2` or `WalkHashed3` of the served backend.
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
        } => walk_hashed3(
            sink,
            id,
            backend,
            &root,
            (algorithm, min_bytes, meta),
            cancel,
        ),
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
/// portions; `meta` asks for listing attributes.
fn walk_hashed3(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    (algorithm, min_bytes, meta): (u8, u64, bool),
    cancel: &AtomicBool,
) -> io::Result<()> {
    check_algorithm(algorithm)?;
    let request = HashWalkRequest {
        algorithm: algorithm_from_wire(algorithm),
        min_bytes,
        meta,
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
    if vfs::supports_hash_walk(&**backend, root)? && relay(items, backend, root, request, cancel)? {
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

struct Walk<'a, 'i> {
    items: &'a mut Items<'i>,
    backend: &'a BackendHandle,
    request: HashWalkRequest,
    cancel: &'a AtomicBool,
    budget: WalkBudget,
}

impl Walk<'_, '_> {
    fn emit(&mut self, item: HashWalkItem) -> io::Result<()> {
        self.items.send(item)
    }

    fn omit(&mut self, rel: String, reason: OmissionReason, detail: String) -> io::Result<()> {
        self.emit(HashWalkItem::Omitted(VfsOmission {
            rel,
            reason,
            detail,
        }))
    }

    /// Depth-first over tolerant listings; only the root's listing failing
    /// fails the walk.
    fn run(&mut self, root: &str) -> io::Result<()> {
        self.budget.record(root, 0)?;
        let mut stack = vec![(root.to_string(), String::new(), 0usize)];
        while let Some((directory, rel_dir, depth)) = stack.pop() {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            let listing = match vfs::list_dir_tolerant(&**self.backend, &directory) {
                Ok(listing) => listing,
                Err(error) if rel_dir.is_empty() => return Err(error),
                Err(error) => {
                    let reason = vfs::omission_reason(&error).unwrap_or(OmissionReason::Unreadable);
                    self.omit(rel_dir, reason, error.to_string())?;
                    continue;
                }
            };
            for omitted in listing.omitted {
                let rel = rel_join(&rel_dir, &omitted.rel);
                self.omit(rel, omitted.reason, omitted.detail)?;
            }
            for entry in listing.entries {
                if self.cancel.load(Ordering::Relaxed) {
                    return Err(canceled());
                }
                let rel = rel_join(&rel_dir, &entry.name);
                if crate::vfs::validate_child_name(&entry.name).is_err() {
                    self.omit(
                        rel,
                        OmissionReason::Unrepresentable,
                        "Name ist kein Pfad".into(),
                    )?;
                    continue;
                }
                let path = join_path(&directory, &entry.name);
                self.budget.record(&path, depth + 1)?;
                if entry.is_symlink {
                    self.omit(rel, OmissionReason::Link, "Verknüpfung".into())?;
                    continue;
                }
                if entry.special {
                    self.omit(
                        rel,
                        OmissionReason::Special,
                        "Pipe, Socket oder Gerät".into(),
                    )?;
                    continue;
                }
                if entry.is_dir {
                    self.emit(HashWalkItem::Entry(vfs::HashWalkEntry {
                        rel: rel.clone(),
                        is_dir: true,
                        size: 0,
                        mtime_ms: entry.mtime_ms,
                        digest: None,
                        meta: None,
                    }))?;
                    stack.push((path, rel, depth + 1));
                    continue;
                }
                if entry.size < self.request.min_bytes {
                    continue;
                }
                let digest = match self.request.algorithm {
                    None => None,
                    // A provider MD5 of the listing needs no download.
                    Some(HashAlgorithm::Md5) if entry.content_md5.is_some() => {
                        entry.content_md5.clone()
                    }
                    Some(algorithm) => {
                        match self.digest(&path, entry.id.as_deref(), entry.size, algorithm) {
                            Ok(digest) => Some(digest),
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                return Err(error)
                            }
                            Err(error) => {
                                let reason = vfs::omission_reason(&error)
                                    .unwrap_or(OmissionReason::Unreadable);
                                self.omit(rel, reason, error.to_string())?;
                                continue;
                            }
                        }
                    }
                };
                self.emit(HashWalkItem::Entry(vfs::HashWalkEntry {
                    rel,
                    is_dir: false,
                    size: entry.size,
                    mtime_ms: entry.mtime_ms,
                    digest,
                    meta: None,
                }))?;
            }
        }
        Ok(())
    }

    /// Lowercase hex digest of the whole content; a length other than the
    /// listed one is a file that changed while it was walked.
    fn digest(
        &self,
        path: &str,
        id: Option<&str>,
        size: u64,
        algorithm: HashAlgorithm,
    ) -> io::Result<String> {
        let mut reader = vfs::open_read_regular(&**self.backend, path, id)?;
        let mut md5 = md5::Context::new();
        let mut sha256 = Context::new(&SHA256);
        let mut buffer = vec![0u8; CHUNK];
        let mut read = 0u64;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            let count = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            read = read.saturating_add(count as u64);
            match algorithm {
                HashAlgorithm::Md5 => md5.consume(&buffer[..count]),
                HashAlgorithm::Sha256 => sha256.update(&buffer[..count]),
            }
        }
        if read != size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Datei hat sich während des Walks geändert",
            ));
        }
        Ok(match algorithm {
            HashAlgorithm::Md5 => format!("{:x}", md5.compute()),
            HashAlgorithm::Sha256 => sha256
                .finish()
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        })
    }
}
