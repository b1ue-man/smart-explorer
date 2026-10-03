//! `WalkHashed2` (`ext-v1`) for the backend the service serves. A backend
//! that walks next to its data (a Share host with `hash_walk_v1`) streams its
//! own items; every other one is walked here: tolerant listings, links,
//! special files and unreadable entries as omissions (never a failed walk),
//! files below the minimum size left out, digests read only where asked.
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use ring::digest::{Context, SHA256};

use crate::agent::ext_wire::{algorithm_from_wire, omission_to_wire};
use crate::agent_proto::{Frame, CHUNK};
use crate::analytics::HashAlgorithm;
use crate::vfs::{
    self as vfs, BackendHandle, HashWalkItem, HashWalkRequest, OmissionReason, VfsOmission,
};

use super::backend_budget::WalkBudget;
use super::backend_server::{emit, Sink};

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

fn item_frame(item: HashWalkItem) -> Frame {
    match item {
        HashWalkItem::Entry(entry) => Frame::HashEntry {
            rel: entry.rel,
            is_dir: entry.is_dir,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            md5: entry.digest,
        },
        HashWalkItem::Omitted(omitted) => Frame::HashOmitted(omission_to_wire(omitted)),
    }
}

pub(super) fn walk_hashed2(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    algorithm: u8,
    min_bytes: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
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
    let request = HashWalkRequest {
        algorithm: algorithm_from_wire(algorithm),
        min_bytes,
    };
    if vfs::supports_hash_walk(&**backend, root)?
        && relay(sink, id, backend, root, request, cancel)?
    {
        return emit(sink, id, &Frame::End);
    }
    let mut walk = Walk {
        sink,
        id,
        backend,
        request,
        cancel,
        budget: WalkBudget::streaming(),
    };
    walk.run(root)?;
    emit(sink, id, &Frame::End)
}

/// Streams the backend's own walk; `false` when it turned out unsupported
/// before its first item (then this side walks).
fn relay(
    sink: &Sink,
    id: u64,
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
                    forwarded = emit(sink, id, &item_frame(item));
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

struct Walk<'a> {
    sink: &'a Sink,
    id: u64,
    backend: &'a BackendHandle,
    request: HashWalkRequest,
    cancel: &'a AtomicBool,
    budget: WalkBudget,
}

impl Walk<'_> {
    fn emit(&self, item: HashWalkItem) -> io::Result<()> {
        emit(self.sink, self.id, &item_frame(item))
    }

    fn omit(&self, rel: String, reason: OmissionReason, detail: String) -> io::Result<()> {
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
