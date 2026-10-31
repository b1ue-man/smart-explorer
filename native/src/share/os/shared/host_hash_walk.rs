//! Host hash walks keep omissions explicit and local traversal handle-confined.
//! Local roots are walked on the host's threads (`host_hash_walk_local.rs`);
//! every folder reaches the stream before the entries below it. Files are
//! opened only for a requested digest: size and time come from the listing.
//!
//! The items keep the order they were produced in: a batch never holds an
//! entry after an omission (the client forwards a batch's entries before
//! its omissions). With `meta` (`hash_walk_meta_v1`) the items of one folder
//! (its entries and omissions, and the omission of the folder itself) form
//! one unbroken run after the folder's own entry: every folder is read whole
//! before the next one starts, and a folder that fills more than one batch
//! holds the walk's run lock until its last batch is sent, so no other
//! thread's batch comes in between.
use crate::{
    analytics::Progress,
    share::{
        fs_access::FsAccess,
        fs_response::{
            FsHashEntry, FsHashMeta, FsHashWalkMessage, FsOmission, FsOmissionReason,
            LIST_BATCH_MAX_BYTES,
        },
        host_list, host_stream,
        wire::{FsHashAlgo, FsHashWalk, FsResponse},
    },
    vfs::Scheme,
};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};
use tokio::sync::mpsc;

#[path = "host_hash_walk_local.rs"]
mod local;

/// Walks deeper than this report the folder as unreadable.
pub(super) const MAX_DEPTH: usize = 2048;

/// What every thread of one walk shares: the request, the stream and the
/// totals the closing `Done` reports.
pub(super) struct Shared<'a> {
    pub(super) request: &'a FsHashWalk,
    pub(super) cancel: &'a AtomicBool,
    tx: &'a mpsc::Sender<io::Result<FsResponse>>,
    pub(super) progress: &'a Progress,
    entries: AtomicU64,
    omitted: AtomicU64,
    /// Every batch is sent under it (see the module head).
    run: Mutex<()>,
}

impl Shared<'_> {
    fn lock_run(&self) -> MutexGuard<'_, ()> {
        self.run
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn check(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(io::ErrorKind::Interrupted.into())
        } else {
            Ok(())
        }
    }

    /// The listing attributes when the request asked for them.
    pub(super) fn meta(&self, meta: impl FnOnce() -> FsHashMeta) -> Option<FsHashMeta> {
        self.request.meta.then(meta)
    }

    pub(super) fn digest(&self, mut read: impl Read, expected: u64) -> io::Result<Option<String>> {
        if self.request.algo.is_none() {
            return Ok(None);
        }
        let mut sha = Sha256::new();
        let mut md5 = md5::Context::new();
        let mut buffer = vec![0; 1024 * 1024];
        let mut size = 0u64;
        loop {
            self.check()?;
            let n = match read.read(&mut buffer) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if n == 0 {
                break;
            }
            size = size.saturating_add(n as u64);
            if size > expected {
                break;
            }
            match self.request.algo {
                Some(FsHashAlgo::Sha256) => sha.update(&buffer[..n]),
                Some(FsHashAlgo::Md5) => md5.consume(&buffer[..n]),
                _ => {}
            }
            self.progress.bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
        if size != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Dateigröße hat sich beim Lesen geändert",
            ));
        }
        Ok(Some(match self.request.algo {
            Some(FsHashAlgo::Sha256) => format!("{:x}", sha.finalize()),
            Some(FsHashAlgo::Md5) => format!("{:x}", md5.compute()),
            _ => return Ok(None),
        }))
    }
}

/// The items one thread produced and has not sent yet.
pub(super) struct Batch<'s, 'a> {
    shared: &'s Shared<'a>,
    entries: Vec<FsHashEntry>,
    omitted: Vec<FsOmission>,
    bytes: usize,
    /// Held from a batch sent in the middle of a folder (`meta`) until that
    /// folder's last batch is out.
    held: Option<MutexGuard<'s, ()>>,
}

impl<'s, 'a> Batch<'s, 'a> {
    pub(super) fn new(shared: &'s Shared<'a>) -> Self {
        Self {
            shared,
            entries: Vec::new(),
            omitted: Vec::new(),
            bytes: 0,
            held: None,
        }
    }

    pub(super) fn shared(&self) -> &'s Shared<'a> {
        self.shared
    }

    pub(super) fn add(&mut self, entry: Result<FsHashEntry, FsOmission>) -> io::Result<()> {
        self.shared.check()?;
        let cost = match &entry {
            Ok(entry) => entry.wire_bytes(),
            Err(hole) => hole.wire_bytes(),
        }
        .saturating_mul(6);
        if cost > LIST_BATCH_MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Prüfsummenpfad überschreitet das Drahtformat",
            ));
        }
        let entry_after_omission = entry.is_ok() && !self.omitted.is_empty();
        if entry_after_omission || self.bytes + cost > LIST_BATCH_MAX_BYTES {
            self.send_within()?;
        }
        self.bytes += cost;
        match entry {
            Ok(entry) => {
                self.entries.push(entry);
                self.shared.entries.fetch_add(1, Ordering::Relaxed);
            }
            Err(hole) => {
                self.omitted.push(hole);
                self.shared.omitted.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(())
    }

    pub(super) fn hole(
        &mut self,
        rel: &str,
        reason: FsOmissionReason,
        detail: impl Into<String>,
    ) -> io::Result<()> {
        self.add(Err(FsOmission {
            rel: rel.into(),
            reason,
            detail: detail.into(),
        }))
    }

    pub(super) fn failed(&mut self, rel: &str, error: io::Error) -> io::Result<()> {
        let reason = if error.kind() == io::ErrorKind::NotFound {
            FsOmissionReason::Vanished
        } else {
            FsOmissionReason::Unreadable
        };
        self.hole(rel, reason, error.to_string())
    }

    /// Sends a batch while the current folder is not done yet; with `meta`
    /// the run lock stays held until `end_folder`.
    fn send_within(&mut self) -> io::Result<()> {
        if self.shared.request.meta && self.held.is_none() {
            self.held = Some(self.shared.lock_run());
        }
        self.emit()
    }

    /// The current folder is done: a run lock held for it is released.
    pub(super) fn end_folder(&mut self) -> io::Result<()> {
        if self.held.is_some() {
            return self.send();
        }
        Ok(())
    }

    /// Sends what is buffered, between folders.
    pub(super) fn send(&mut self) -> io::Result<()> {
        let result = if self.held.is_some() || (self.entries.is_empty() && self.omitted.is_empty())
        {
            self.emit()
        } else {
            let shared = self.shared;
            let _run = shared.lock_run();
            self.emit()
        };
        self.held = None;
        result
    }

    fn emit(&mut self) -> io::Result<()> {
        self.bytes = 0;
        if self.entries.is_empty() && self.omitted.is_empty() {
            return Ok(());
        }
        host_stream::emit(
            self.shared.tx,
            self.shared.cancel,
            FsResponse::HashWalk {
                message: FsHashWalkMessage::Batch {
                    entries: std::mem::take(&mut self.entries),
                    omitted: std::mem::take(&mut self.omitted),
                },
            },
        )
    }
}

impl crate::agent_proto::WalkOutput for Batch<'_, '_> {
    fn flush(&mut self) -> io::Result<()> {
        self.send()
    }
}

pub(super) fn child_rel(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.into()
    } else {
        format!("{rel}/{name}")
    }
}

pub(in crate::share) fn run(
    request: FsHashWalk,
    access: FsAccess,
    tx: mpsc::Sender<io::Result<FsResponse>>,
    cancel: Arc<AtomicBool>,
    p: Progress,
) -> io::Result<()> {
    if request.single_file {
        return crate::share::host_hash_file::run(request, access, tx, cancel, p);
    }
    if request.algo == Some(FsHashAlgo::Unknown) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Hashalgorithmus nicht unterstützt",
        ));
    }
    let shared = Shared {
        request: &request,
        cancel: &cancel,
        tx: &tx,
        progress: &p,
        entries: AtomicU64::new(0),
        omitted: AtomicU64::new(0),
        run: Mutex::new(()),
    };
    let mut batch = Batch::new(&shared);
    visit(&mut batch, &request.path, &access, "")?;
    batch.send()?;
    host_stream::emit(
        &tx,
        &cancel,
        FsResponse::HashWalk {
            message: FsHashWalkMessage::Done {
                entries: shared.entries.load(Ordering::Relaxed),
                omitted: shared.omitted.load(Ordering::Relaxed),
            },
        },
    )
}

fn visit(batch: &mut Batch<'_, '_>, path: &str, access: &FsAccess, rel: &str) -> io::Result<()> {
    let parts = crate::share::fs::split_clean(path)?;
    if access.is_dynamic() && (parts.is_empty() || parts == ["Verbindungen"]) {
        // The container's entries first, then their content (one run each).
        let mut children = Vec::new();
        for entry in access.list_dir(path)? {
            crate::vfs::validate_child_name(&entry.name)?;
            let child = child_rel(rel, &entry.name);
            let meta = batch.shared().meta(FsHashMeta::default);
            batch.add(Ok(FsHashEntry {
                rel: child.clone(),
                is_dir: true,
                meta,
                ..FsHashEntry::default()
            }))?;
            children.push((
                format!("{}/{}", path.trim_end_matches('/'), entry.name),
                child,
            ));
        }
        batch.end_folder()?;
        for (child_path, child) in children {
            visit(batch, &child_path, access, &child)?;
        }
        return Ok(());
    }
    let target = match access.resolve(path) {
        Ok(target) => target,
        Err(error) => {
            batch.failed(rel, error)?;
            return batch.end_folder();
        }
    };
    if target.backend.scheme() == Scheme::Local {
        let physical = crate::share::storage_roots::local_path(&target)?;
        if crate::share::storage_roots::is_private(&physical) {
            batch.hole(
                rel,
                FsOmissionReason::Unreadable,
                "Interner Host-Speicher ist geschützt",
            )?;
            return batch.end_folder();
        }
        match crate::share::storage_roots::open_local(&target) {
            Ok(handle) => {
                // Folders above this root (dynamic containers) go out first.
                batch.send()?;
                local::walk(batch.shared(), handle, physical, rel)
            }
            Err(e) => {
                batch.failed(rel, e)?;
                batch.end_folder()
            }
        }
    } else {
        remote(batch, &*target.backend, &target.path, rel, 0)
    }
}

/// An exported connection's folder, one listing after the other (its own
/// connection bounds the pace): the folder's items, then its subfolders.
fn remote(
    batch: &mut Batch<'_, '_>,
    backend: &dyn crate::vfs::Backend,
    path: &str,
    rel: &str,
    depth: usize,
) -> io::Result<()> {
    let subfolders = remote_folder(batch, backend, path, rel, depth)?;
    batch.end_folder()?;
    for (full, child) in subfolders {
        remote(batch, backend, &full, &child, depth + 1)?;
    }
    Ok(())
}

/// The items of one folder of an exported connection; returns its
/// subfolders (path, rel) to walk next.
fn remote_folder(
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
            let digest = if batch.shared().request.algo.is_none() {
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
