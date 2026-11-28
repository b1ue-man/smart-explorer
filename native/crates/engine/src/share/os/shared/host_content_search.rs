//! Content search on the Share host (`content_search_v1`, plan
//! docs/plaene/2026-11-27-dateien-suche-ansicht section B): the files are
//! searched on the device that holds them, within the export's rights, and
//! only the hits travel.
//!
//! The rights apply before anything is read: every folder is entered and
//! every file opened only after the session's right was checked again
//! (`FsAccess::check_read`); the Share-private, trash, engine and stage
//! names (`host_list::hidden`) and the host's own storage
//! (`storage_roots::excluded`) are neither read nor reported; links and
//! special files are never followed or read. Local roots are searched on the
//! host's threads through held folder handles (`host_content_search_local.rs`),
//! exported connections by the backend's own search
//! (`host_content_search_remote.rs`).
//!
//! The stream carries the events in portions of at most
//! `LIST_BATCH_MAX_BYTES` (estimated), sent at least every `FLUSH_AFTER`,
//! and closes with totals whose `hits` and `skipped` are exactly what the
//! portions carried. `max_hits` holds over the whole search: the event that
//! reaches it is cut there and ends its file (`last`), and every walker
//! stops.
use crate::{
    agent_proto::{ContentEvent, ContentMatcher, ContentQuery, PathFilter},
    share::{
        fs_access::FsAccess,
        fs_response::{
            FsContentEvent, FsContentMessage, FsContentSummary, FsLineHit, FsSkipKind,
            LIST_BATCH_MAX_BYTES,
        },
        host_list, host_stream, storage_roots,
        wire::{FsContentSearch, FsResponse},
    },
    vfs::Scheme,
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

#[path = "host_content_search_local.rs"]
mod local;
#[path = "host_content_search_remote.rs"]
mod remote;
#[cfg(test)]
#[path = "host_content_search_task_tests.rs"]
mod task_tests;

/// Searches deeper than this report the folder as unlistable.
pub(super) const MAX_DEPTH: usize = 2048;
/// Buffered events go out at least this often, so hits show while the host
/// still reads.
const FLUSH_AFTER: Duration = Duration::from_millis(200);

/// Running totals the heartbeats report.
#[derive(Default)]
pub(in crate::share) struct Totals {
    files: AtomicU64,
    bytes: AtomicU64,
    /// Hit lines sent (context lines not counted).
    hits: AtomicU64,
}

impl Totals {
    pub(in crate::share) fn progress(&self) -> FsResponse {
        FsResponse::ContentSearch {
            message: FsContentMessage::Progress {
                files: self.files.load(Ordering::Relaxed),
                bytes: self.bytes.load(Ordering::Relaxed),
                hits: self.hits.load(Ordering::Relaxed),
            },
        }
    }

    pub(super) fn searched(&self, files: u64, bytes: u64) {
        self.files.fetch_add(files, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// What every thread of one search shares.
pub(super) struct Shared<'a> {
    pub(super) query: &'a ContentQuery,
    pub(super) access: &'a FsAccess,
    cancel: &'a AtomicBool,
    /// Every walker stops: at `max_hits`, when the stream ended or a portion
    /// could not be sent.
    pub(super) halt: AtomicBool,
    limited: AtomicBool,
    tx: &'a mpsc::Sender<io::Result<FsResponse>>,
    pub(super) totals: &'a Totals,
    matched: AtomicU64,
    skipped: AtomicU64,
}

impl Shared<'_> {
    /// Before a folder is entered or a file opened: the stream still runs
    /// and the session still holds its right.
    pub(super) fn check(&self) -> io::Result<()> {
        let ended = self.halt.load(Ordering::Relaxed) && !self.limited();
        if self.cancel.load(Ordering::Relaxed) || ended {
            self.halt.store(true, Ordering::Relaxed);
            return Err(io::ErrorKind::Interrupted.into());
        }
        self.access.check_read()
    }

    /// The search reached `max_hits`: walkers end without error.
    pub(super) fn limited(&self) -> bool {
        self.limited.load(Ordering::Relaxed)
    }

    /// A backend's own search ended at `max_hits`.
    pub(super) fn reach_limit(&self) {
        self.limited.store(true, Ordering::Relaxed);
        self.halt.store(true, Ordering::Relaxed);
    }

    /// Hit lines `max_hits` still admits (`u64::MAX` without a limit).
    pub(super) fn remaining(&self) -> u64 {
        match self.query.max_hits {
            0 => u64::MAX,
            max => max.saturating_sub(self.totals.hits.load(Ordering::Relaxed)),
        }
    }

    /// Readers stop at once (limit, end of the stream, failed portion).
    pub(super) fn stopped(&self) -> bool {
        self.halt.load(Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed)
    }

    /// An error of a walker or of the backend's search after the search
    /// reached `max_hits` (and the stream still runs) is its regular end.
    pub(super) fn ended_at_limit(&self) -> bool {
        self.limited() && !self.cancel.load(Ordering::Relaxed)
    }

    /// Takes up to `wanted` hit lines from the budget of `max_hits`; the one
    /// that exhausts it sets the limit.
    fn reserve(&self, wanted: u64) -> u64 {
        let max = self.query.max_hits;
        if max == 0 {
            self.totals.hits.fetch_add(wanted, Ordering::Relaxed);
            return wanted;
        }
        let mut granted = 0;
        let mut reached = false;
        let _ = self
            .totals
            .hits
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |sent| {
                granted = wanted.min(max.saturating_sub(sent));
                reached = sent + granted >= max;
                Some(sent + granted)
            });
        if reached {
            self.limited.store(true, Ordering::Relaxed);
            self.halt.store(true, Ordering::Relaxed);
        }
        granted
    }
}

/// The events one thread produced and has not sent yet.
pub(super) struct Sink<'s, 'a> {
    shared: &'s Shared<'a>,
    events: Vec<FsContentEvent>,
    bytes: usize,
    since: Instant,
    failed: Option<io::Error>,
    /// A `Skipped` of the file being read went out (`begin_file`).
    skipped_file: bool,
}

impl<'s, 'a> Sink<'s, 'a> {
    pub(super) fn new(shared: &'s Shared<'a>) -> Self {
        Self {
            shared,
            events: Vec::new(),
            bytes: 0,
            since: Instant::now(),
            failed: None,
            skipped_file: false,
        }
    }

    pub(super) fn shared(&self) -> &'s Shared<'a> {
        self.shared
    }

    /// One event of the search core (`search_reader`, a backend's search);
    /// its totals travel as the stream's own `Progress`.
    pub(super) fn event(&mut self, event: ContentEvent) {
        if let Some(event) = FsContentEvent::from_event(event) {
            self.wire(event);
        }
    }

    pub(super) fn skip(&mut self, rel: &str, reason: FsSkipKind, detail: impl ToString) {
        self.wire(FsContentEvent::Skipped {
            rel: rel.to_string(),
            reason,
            detail: detail.to_string(),
        });
    }

    pub(super) fn begin_file(&mut self) {
        self.skipped_file = false;
    }

    pub(super) fn file_skipped(&self) -> bool {
        self.skipped_file
    }

    fn wire(&mut self, event: FsContentEvent) {
        if self.failed.is_some() || event.rel().split('/').any(host_list::hidden) {
            return;
        }
        match event {
            FsContentEvent::Hits {
                rel,
                size,
                mtime_ms,
                mut lines,
                mut last,
            } => {
                if self.shared.limited() {
                    return;
                }
                let wanted = lines.iter().filter(|line| !line.context).count() as u64;
                let granted = self.shared.reserve(wanted);
                if granted < wanted {
                    if granted == 0 {
                        return;
                    }
                    lines.truncate(cut(&lines, granted));
                    last = true;
                }
                if last {
                    self.shared.matched.fetch_add(1, Ordering::Relaxed);
                }
                self.hits(rel, size, mtime_ms, lines, last);
            }
            skipped @ FsContentEvent::Skipped { .. } => {
                self.skipped_file = true;
                self.shared.skipped.fetch_add(1, Ordering::Relaxed);
                let cost = skipped.wire_bytes();
                self.push(skipped, cost);
            }
        }
    }

    /// The lines of one file in as many events as the portion size needs.
    fn hits(&mut self, rel: String, size: u64, mtime_ms: i64, lines: Vec<FsLineHit>, last: bool) {
        let head = rel.len().saturating_mul(6).saturating_add(96);
        let mut part = Vec::new();
        let mut cost = head;
        for line in lines {
            let line_cost = line.wire_bytes();
            if !part.is_empty() && cost.saturating_add(line_cost) > LIST_BATCH_MAX_BYTES {
                let event = FsContentEvent::Hits {
                    rel: rel.clone(),
                    size,
                    mtime_ms,
                    lines: std::mem::take(&mut part),
                    last: false,
                };
                self.push(event, cost);
                cost = head;
            }
            cost = cost.saturating_add(line_cost);
            part.push(line);
        }
        let event = FsContentEvent::Hits {
            rel,
            size,
            mtime_ms,
            lines: part,
            last,
        };
        self.push(event, cost);
    }

    fn push(&mut self, event: FsContentEvent, cost: usize) {
        let full = self.bytes.saturating_add(cost) > LIST_BATCH_MAX_BYTES;
        if !self.events.is_empty() && (full || self.since.elapsed() >= FLUSH_AFTER) {
            if let Err(error) = self.send() {
                self.failed = Some(error);
                self.shared.halt.store(true, Ordering::Relaxed);
                return;
            }
        }
        self.bytes = self.bytes.saturating_add(cost);
        self.events.push(event);
    }

    /// Sends what is buffered once `FLUSH_AFTER` passed since the last portion.
    pub(super) fn tick(&mut self) -> io::Result<()> {
        if !self.events.is_empty() && self.since.elapsed() >= FLUSH_AFTER {
            return self.send();
        }
        self.failure()
    }

    /// A portion that could not be sent ends the search with its error.
    pub(super) fn failure(&mut self) -> io::Result<()> {
        match self.failed.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Sends what is buffered.
    pub(super) fn send(&mut self) -> io::Result<()> {
        self.failure()?;
        self.since = Instant::now();
        self.bytes = 0;
        if self.events.is_empty() {
            return Ok(());
        }
        host_stream::emit(
            self.shared.tx,
            self.shared.cancel,
            FsResponse::ContentSearch {
                message: FsContentMessage::Events {
                    events: std::mem::take(&mut self.events),
                },
            },
        )
    }
}

impl crate::agent_proto::WalkOutput for Sink<'_, '_> {
    fn flush(&mut self) -> io::Result<()> {
        self.send()
    }
}

/// The length of `lines` that keeps `granted` hit lines (and the context
/// lines before the last of them).
fn cut(lines: &[FsLineHit], granted: u64) -> usize {
    let mut kept = 0;
    for (index, line) in lines.iter().enumerate() {
        if !line.context {
            kept += 1;
            if kept == granted {
                return index + 1;
            }
        }
    }
    lines.len()
}

pub(super) fn child_rel(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.into()
    } else {
        format!("{rel}/{name}")
    }
}

/// The query's pattern or filters the host cannot read answer the request.
pub(super) fn compiled(query: &ContentQuery) -> io::Result<(ContentMatcher, PathFilter)> {
    let invalid = |error: String| io::Error::new(io::ErrorKind::InvalidInput, error);
    Ok((
        ContentMatcher::new(query).map_err(invalid)?,
        PathFilter::new(query).map_err(invalid)?,
    ))
}

pub(in crate::share) fn run(
    request: FsContentSearch,
    access: FsAccess,
    tx: mpsc::Sender<io::Result<FsResponse>>,
    cancel: Arc<AtomicBool>,
    totals: Arc<Totals>,
) -> io::Result<()> {
    let query = request.query.to_query()?;
    let (_, filter) = compiled(&query)?;
    let shared = Shared {
        query: &query,
        access: &access,
        cancel: &cancel,
        halt: AtomicBool::new(false),
        limited: AtomicBool::new(false),
        tx: &tx,
        totals: &totals,
        matched: AtomicU64::new(0),
        skipped: AtomicU64::new(0),
    };
    let mut sink = Sink::new(&shared);
    match visit(&mut sink, &filter, &request.path, "", 0) {
        Err(error) if !shared.ended_at_limit() => return Err(error),
        _ => {}
    }
    sink.send()?;
    let summary = FsContentSummary {
        files_searched: totals.files.load(Ordering::Relaxed),
        files_matched: shared.matched.load(Ordering::Relaxed),
        hits: totals.hits.load(Ordering::Relaxed),
        bytes: totals.bytes.load(Ordering::Relaxed),
        skipped: shared.skipped.load(Ordering::Relaxed),
        limited: shared.limited(),
    };
    host_stream::emit(
        &tx,
        &cancel,
        FsResponse::ContentSearch {
            message: FsContentMessage::Done { summary },
        },
    )
}

/// The folder `path` (`rel` below the search root, at `level`): a dynamic
/// container's exports one after the other, a local root on the host's
/// threads, an exported connection through its backend. The search root
/// itself answers its failure; a folder below it is reported unlistable.
fn visit(
    sink: &mut Sink<'_, '_>,
    filter: &PathFilter,
    path: &str,
    rel: &str,
    level: usize,
) -> io::Result<()> {
    let shared = sink.shared();
    shared.check()?;
    if shared.limited() {
        return Ok(());
    }
    let parts = crate::share::fs::split_clean(path)?;
    if shared.access.is_dynamic() && (parts.is_empty() || parts == ["Verbindungen"]) {
        // A container holds exports (folders) only.
        if !shared.query.subfolders {
            return Ok(());
        }
        let mut children = Vec::new();
        for entry in shared.access.list_dir(path)? {
            crate::vfs::validate_child_name(&entry.name)?;
            let child = child_rel(rel, &entry.name);
            if filter.admit_folder(&child, level + 1, &entry.name, entry.hidden) {
                let full = format!("{}/{}", path.trim_end_matches('/'), entry.name);
                children.push((full, child));
            }
        }
        for (full, child) in children {
            visit(sink, filter, &full, &child, level + 1)?;
        }
        return Ok(());
    }
    let target = match shared.access.resolve(path) {
        Ok(target) => target,
        Err(error) => return unlisted(sink, rel, error),
    };
    if target.backend.scheme() != Scheme::Local {
        return remote::search(sink, filter, &*target.backend, &target.path, rel, level);
    }
    let physical = match storage_roots::local_path(&target) {
        Ok(physical) => physical,
        Err(error) => return unlisted(sink, rel, error),
    };
    if storage_roots::is_private(&physical) {
        // The host's own storage is no Share content: not even reported.
        if rel.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Interner Host-Speicher ist geschützt",
            ));
        }
        return Ok(());
    }
    match storage_roots::open_local(&target) {
        Ok(handle) => {
            sink.send()?;
            local::search(shared, handle, physical, rel, level)
        }
        Err(error) => unlisted(sink, rel, error),
    }
}

/// The search root answers its failure; a folder below it is reported
/// unlistable and the search goes on.
fn unlisted(sink: &mut Sink<'_, '_>, rel: &str, error: io::Error) -> io::Result<()> {
    if rel.is_empty() {
        return Err(error);
    }
    sink.skip(rel, FsSkipKind::UnlistableFolder, error);
    Ok(())
}
