//! Host hash walks keep omissions explicit and local traversal handle-confined.
//! Local roots are walked on the host's threads (`host_hash_walk_local.rs`),
//! exported connections one listing after the other (`host_hash_walk_remote.rs`);
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
//!
//! With `hash_walk_filter_v1` the request's scope applies: digests are read
//! from the files (`fresh`), and with `meta` the job's filter
//! (`agent_proto::walk_filter`) leaves files without a digest and folders
//! unwalked, both still entries. A filter the host cannot compile is no
//! filter: the walk runs unfiltered (as the agent's), never fails for it.
use crate::{
    analytics::Progress,
    share::{
        fs_access::FsAccess,
        fs_response::{
            FsHashEntry, FsHashMeta, FsHashWalkMessage, FsOmission, FsOmissionReason,
            LIST_BATCH_MAX_BYTES,
        },
        host_stream,
        wire::{FsHashAlgo, FsHashWalk, FsResponse},
    },
    vfs::Scheme,
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};
use tokio::sync::mpsc;

#[path = "host_hash_walk_digest.rs"]
mod digest;
#[path = "host_hash_walk_keys.rs"]
mod keys;
#[path = "host_hash_walk_local.rs"]
mod local;
#[path = "host_hash_walk_remote.rs"]
mod remote;
#[path = "host_hash_walk_runs.rs"]
mod runs;

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
    /// `keys`: the folder runs go through the walk's key tree (`keys.rs`).
    keys: Option<crate::agent_proto::KeyTree<keys::Item>>,
    /// `hash_walk_filter_v1` with `meta`: the job's filter
    /// (`agent_proto::walk_filter`): no digest for a file it leaves out, no
    /// walk into a folder it leaves out, both still entries.
    filter: Option<crate::agent_proto::HostFilter>,
    /// `hash_walk_filter_v1`: digests are read from the files, never taken
    /// from the host's digest cache.
    pub(super) fresh: bool,
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

    pub(super) fn filter(&self) -> Option<&crate::agent_proto::HostFilter> {
        self.filter.as_ref()
    }
}

/// The compiled filter and the fresh flag of a request's scope; the filter
/// only with `meta` (the `hidden` it tests is the one the entries carry). A
/// filter this host cannot compile (a pattern of a later parser, a set over
/// its limits) is no filter: the walk runs unfiltered, as the agent's does,
/// and the client applies its own filter to everything it gets.
fn scope_of(request: &FsHashWalk) -> (Option<crate::agent_proto::HostFilter>, bool) {
    let Some(scope) = &request.scope else {
        return (None, false);
    };
    let filter = scope
        .filter
        .clone()
        .filter(|_| request.meta)
        .and_then(|filter| {
            crate::agent_proto::WalkFilterSpec::from(filter)
                .compile()
                .ok()
        });
    (filter, scope.fresh)
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
    /// The run of the current folder of a keyed walk.
    run: Option<crate::agent_proto::RunKeys<keys::Item>>,
}

impl<'s, 'a> Batch<'s, 'a> {
    pub(super) fn new(shared: &'s Shared<'a>) -> Self {
        Self {
            shared,
            entries: Vec::new(),
            omitted: Vec::new(),
            bytes: 0,
            held: None,
            run: None,
        }
    }

    /// The folder `rel` starts (keyed walks track every folder's run).
    pub(super) fn begin_folder(&mut self, rel: &str) -> io::Result<()> {
        if self.shared.keys.is_some() {
            return keys::begin(self, rel);
        }
        Ok(())
    }

    pub(super) fn shared(&self) -> &'s Shared<'a> {
        self.shared
    }

    pub(super) fn add(&mut self, entry: Result<FsHashEntry, FsOmission>) -> io::Result<()> {
        self.shared.check()?;
        if self.shared.keys.is_some() {
            return keys::add(self, entry);
        }
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

    /// A folder the walk's filter leaves out: its entry, never its content;
    /// a keyed walk gives it no key (`agent_proto::RunKeys::skip_dir`).
    pub(super) fn skipped_dir(&mut self, entry: FsHashEntry) -> io::Result<()> {
        let rel = entry.rel.clone();
        self.add(Ok(entry))?;
        if let Some(run) = self.run.as_mut() {
            if let Some(name) = run.name_of(&rel)? {
                run.skip_dir(name);
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

    /// A folder begun here is read on another thread (`runs.rs`).
    pub(super) fn abandon_folder(&mut self) {
        self.run = None;
    }

    /// The current folder is done: a run lock held for it is released.
    pub(super) fn end_folder(&mut self) -> io::Result<()> {
        if self.shared.keys.is_some() {
            return keys::end(self);
        }
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
    if request.algo.is_some() {
        crate::agent_proto::hash_cache::configure_once(digest::cache_folder);
    }
    let (filter, fresh) = scope_of(&request);
    let shared = Shared {
        request: &request,
        cancel: &cancel,
        tx: &tx,
        progress: &p,
        entries: AtomicU64::new(0),
        omitted: AtomicU64::new(0),
        run: Mutex::new(()),
        keys: keys::tree(&request)?,
        filter,
        fresh,
    };
    let mut batch = Batch::new(&shared);
    runs::visit(&mut batch, &request, &access)?;
    batch.send()?;
    if request.algo.is_some() {
        // The digests this walk took are written once, at its end.
        crate::agent_proto::hash_cache::flush_now();
    }
    if shared.keys.as_ref().is_some_and(|tree| !tree.complete()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Walk endete vor dem letzten Ordner",
        ));
    }
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
        batch.begin_folder(rel)?;
        let mut children = Vec::new();
        for entry in access.list_dir(path)? {
            crate::vfs::validate_child_name(&entry.name)?;
            let child = child_rel(rel, &entry.name);
            let meta = batch.shared().meta(FsHashMeta::default);
            let row = FsHashEntry {
                rel: child.clone(),
                is_dir: true,
                meta,
                ..FsHashEntry::default()
            };
            let filter = batch.shared().filter();
            if filter.is_some_and(|f| f.leaves_out_dir(&child, false)) {
                batch.skipped_dir(row)?;
                continue;
            }
            batch.add(Ok(row))?;
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
            batch.begin_folder(rel)?;
            batch.failed(rel, error)?;
            return batch.end_folder();
        }
    };
    if target.backend.scheme() == Scheme::Local {
        let physical = crate::share::storage_roots::local_path(&target)?;
        if crate::share::storage_roots::is_private(&physical) {
            batch.begin_folder(rel)?;
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
                batch.begin_folder(rel)?;
                batch.failed(rel, e)?;
                batch.end_folder()
            }
        }
    } else {
        remote::walk(batch, &*target.backend, &target.path, rel, 0)
    }
}
