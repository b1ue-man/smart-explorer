//! "Replace in Files" through the storage interface (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C „Wo ersetzt wird“):
//! the files are found where they lie (`content_search`: locally, in the SSH
//! agent, on the Share host, otherwise by reading through the backend), or
//! taken from a selection of a search result; each file is then read, its
//! matches replaced in memory (`agent_proto::ReplaceProgram`), its original
//! kept encrypted (`ReplaceJournal`) and the new content published through
//! the backend of the place (`content_replace_write.rs`; a Share host checks
//! its write rights there). Local files are replaced on every core, remote
//! ones one after the other. A cancel ends the run between two files; files
//! written before stay written and stay in the journal.
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::agent_proto::{
    ContentEvent, HitRef, ReplaceEmit, ReplaceEvent, ReplaceProgram, ReplaceRequest,
    ReplaceSelection, ReplaceSkip, ReplaceSummary, SkipReason,
};

use super::content_replace_write::{process, Candidate};
use super::replace_journal::{ReplaceJournal, RunWriter};
use super::{Backend, VfsResult};

/// Shortest time between two progress events.
const PROGRESS_EVERY_MS: u64 = 250;
/// How often the reader of the search's events looks at the cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Replaces the matches `request` names below `root` of `backend`; the
/// originals go into the user's journal (`ReplaceJournal::open_default`).
/// `location` is the place as the surface names it (se target grammar), kept
/// for `ReplaceJournal::undo`. `Err` for an invalid query, an unusable
/// journal (before anything changes), an unlistable root or a failed search
/// that changed nothing; a cancel returns the totals with `canceled`.
pub fn content_replace(
    backend: &dyn Backend,
    root: &str,
    request: &ReplaceRequest,
    location: &str,
    emit: ReplaceEmit<'_>,
    cancel: &AtomicBool,
) -> VfsResult<ReplaceSummary> {
    let program = prepare(request)?;
    if request.dry_run {
        return drive(
            None,
            &program,
            (backend, root, location),
            request,
            emit,
            cancel,
        );
    }
    let journal = ReplaceJournal::open_default()?;
    drive(
        Some(&journal),
        &program,
        (backend, root, location),
        request,
        emit,
        cancel,
    )
}

/// `content_replace` with the originals in `journal`.
pub fn content_replace_with(
    journal: &ReplaceJournal,
    backend: &dyn Backend,
    root: &str,
    request: &ReplaceRequest,
    location: &str,
    emit: ReplaceEmit<'_>,
    cancel: &AtomicBool,
) -> VfsResult<ReplaceSummary> {
    let program = prepare(request)?;
    let journal = (!request.dry_run).then_some(journal);
    drive(
        journal,
        &program,
        (backend, root, location),
        request,
        emit,
        cancel,
    )
}

fn prepare(request: &ReplaceRequest) -> VfsResult<ReplaceProgram> {
    if request.selection == ReplaceSelection::All {
        // Filters and folder regexes, as the search checks them.
        crate::agent_proto::prepare_content_search(&request.query)?;
    }
    ReplaceProgram::new(&request.query, &request.replacement)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

/// The shared state of one running replacement.
pub(super) struct Run<'a> {
    pub(super) backend: &'a dyn Backend,
    pub(super) root: &'a str,
    pub(super) root_is_file: bool,
    pub(super) program: &'a ReplaceProgram,
    pub(super) request: &'a ReplaceRequest,
    location: &'a str,
    journal: Option<&'a ReplaceJournal>,
    writer: Mutex<Option<Result<Arc<RunWriter>, String>>>,
    emit: ReplaceEmit<'a>,
    cancel: &'a AtomicBool,
    gone: AtomicBool,
    /// Files are counted as they are read (a selection; else the search counts).
    count_reads: bool,
    searched: AtomicU64,
    changed: AtomicU64,
    replacements: AtomicU64,
    skipped: AtomicU64,
    started: Instant,
    last_progress: AtomicU64,
}

impl Run<'_> {
    /// Canceled, or the receiver is gone.
    pub(super) fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.gone.load(Ordering::Relaxed)
    }

    pub(super) fn send(&self, event: ReplaceEvent) -> bool {
        if self.gone.load(Ordering::Relaxed) {
            return false;
        }
        let delivered = (self.emit)(event);
        if !delivered {
            self.gone.store(true, Ordering::Relaxed);
        }
        delivered
    }

    pub(super) fn skip(&self, rel: &str, reason: ReplaceSkip) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
        self.send(ReplaceEvent::Skipped {
            rel: rel.to_string(),
            reason,
        });
    }

    /// A file was read (counted where no search counts).
    pub(super) fn read_one(&self) {
        if self.count_reads {
            self.searched.fetch_add(1, Ordering::Relaxed);
            self.progress();
        }
    }

    pub(super) fn changed(&self, rel: &str, replacements: u64, sizes: (u64, u64)) {
        self.changed.fetch_add(1, Ordering::Relaxed);
        self.replacements.fetch_add(replacements, Ordering::Relaxed);
        self.send(ReplaceEvent::File {
            rel: rel.to_string(),
            replacements,
            size_before: sizes.0,
            size_after: sizes.1,
        });
        self.progress();
    }

    fn progress(&self) {
        let now = self.started.elapsed().as_millis() as u64;
        let last = self.last_progress.load(Ordering::Relaxed);
        if now < last + PROGRESS_EVERY_MS
            || self
                .last_progress
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return;
        }
        self.send(ReplaceEvent::Progress {
            files_searched: self.searched.load(Ordering::Relaxed),
            files_changed: self.changed.load(Ordering::Relaxed),
            replacements: self.replacements.load(Ordering::Relaxed),
        });
    }

    /// The backend path of `rel`.
    pub(super) fn path(&self, rel: &str) -> String {
        if self.root_is_file || rel.is_empty() {
            self.root.to_string()
        } else if self.root.ends_with('/') {
            format!("{}{rel}", self.root)
        } else {
            format!("{}/{rel}", self.root)
        }
    }

    /// The journal record of this run, started with the first backup.
    pub(super) fn writer(&self) -> Result<Arc<RunWriter>, String> {
        let Some(journal) = self.journal else {
            return Err("kein Ersetzungsjournal".to_string());
        };
        let mut slot = self
            .writer
            .lock()
            .map_err(|_| "Ersetzungsjournal gesperrt".to_string())?;
        if slot.is_none() {
            let started = journal
                .begin(
                    self.location,
                    self.root,
                    &self.request.query,
                    &self.request.replacement,
                )
                .map(Arc::new)
                .map_err(|error| error.to_string());
            *slot = Some(started);
        }
        slot.clone()
            .unwrap_or_else(|| Err("kein Ersetzungsjournal".to_string()))
    }

    fn summary(&self) -> ReplaceSummary {
        let writer = self.writer.lock().ok().and_then(|slot| slot.clone());
        let files_changed = self.changed.load(Ordering::Relaxed);
        let replacements = self.replacements.load(Ordering::Relaxed);
        let undo_id = match writer {
            Some(Ok(writer)) => writer.finish(files_changed, replacements),
            _ => None,
        };
        ReplaceSummary {
            files_searched: self.searched.load(Ordering::Relaxed),
            files_changed,
            replacements,
            skipped: self.skipped.load(Ordering::Relaxed),
            undo_id,
            canceled: self.stopped(),
        }
    }
}

fn drive(
    journal: Option<&ReplaceJournal>,
    program: &ReplaceProgram,
    (backend, root, location): (&dyn Backend, &str, &str),
    request: &ReplaceRequest,
    emit: ReplaceEmit<'_>,
    cancel: &AtomicBool,
) -> VfsResult<ReplaceSummary> {
    let root_is_file = backend
        .stat(root)
        .is_ok_and(|meta| !meta.is_dir && !meta.is_symlink && !meta.special);
    let run = Run {
        backend,
        root,
        root_is_file,
        program,
        request,
        location,
        journal,
        writer: Mutex::new(None),
        emit,
        cancel,
        gone: AtomicBool::new(false),
        count_reads: request.selection != ReplaceSelection::All,
        searched: AtomicU64::new(0),
        changed: AtomicU64::new(0),
        replacements: AtomicU64::new(0),
        skipped: AtomicU64::new(0),
        started: Instant::now(),
        last_progress: AtomicU64::new(0),
    };
    let threads = if backend.is_local() {
        rayon::current_num_threads().max(1)
    } else {
        1
    };
    let searched = std::thread::scope(|scope| {
        let (todo, work) = crossbeam_channel::bounded::<Candidate>(threads * 4);
        for _ in 0..threads {
            let work = work.clone();
            let run = &run;
            scope.spawn(move || {
                for candidate in work {
                    if !run.stopped() {
                        process(run, candidate);
                    }
                }
            });
        }
        drop(work);
        match &request.selection {
            ReplaceSelection::All => search(&run, todo),
            ReplaceSelection::Files(files) => {
                for rel in files {
                    let candidate = Candidate {
                        rel: rel.clone(),
                        listed: None,
                        hits: None,
                    };
                    if run.stopped() || todo.send(candidate).is_err() {
                        break;
                    }
                }
                Ok(None)
            }
            ReplaceSelection::Hits(hits) => {
                for (rel, hits) in by_file(hits) {
                    let listed = hits.first().map(|hit| (hit.size, hit.mtime_ms));
                    let candidate = Candidate {
                        rel,
                        listed,
                        hits: Some(hits),
                    };
                    if run.stopped() || todo.send(candidate).is_err() {
                        break;
                    }
                }
                Ok(None)
            }
        }
    });
    match searched {
        Ok(Some(files)) => run.searched.store(files, Ordering::Relaxed),
        Ok(None) => {}
        Err(_) if run.stopped() => {}
        Err(error) if run.changed.load(Ordering::Relaxed) == 0 => {
            // Nothing changed: the run is dropped with its (empty) journal record.
            let _ = run.summary();
            return Err(error);
        }
        // Files were changed before the search failed: they stay in the journal.
        Err(error) => run.skip("", ReplaceSkip::Unreadable(error.to_string())),
    }
    Ok(run.summary())
}

/// The search of the run's query (every file with a match, once): its hits
/// go to the workers, its skips and progress to the run's receiver.
fn search(run: &Run<'_>, todo: crossbeam_channel::Sender<Candidate>) -> VfsResult<Option<u64>> {
    let mut query = run.request.query.clone();
    query.files_only = true;
    query.max_hits = 0;
    query.context_lines = 0;
    let halt = AtomicBool::new(run.stopped());
    let (events, received) = crossbeam_channel::bounded::<ContentEvent>(1024);
    let searched = std::thread::scope(|scope| {
        let halt = &halt;
        scope.spawn(move || {
            let mut seen = HashSet::new();
            loop {
                match received.recv_timeout(CANCEL_POLL) {
                    Ok(event) => forward(run, event, &mut seen, &todo, halt),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
                if run.stopped() {
                    halt.store(true, Ordering::Relaxed);
                }
            }
        });
        super::content_search(run.backend, run.root, &query, events, halt)
    });
    searched.map(|summary| Some(summary.files_searched))
}

/// One event of the search: a file with a match to the workers (once), a
/// skip and the progress to the run's receiver.
fn forward(
    run: &Run<'_>,
    event: ContentEvent,
    seen: &mut HashSet<String>,
    todo: &crossbeam_channel::Sender<Candidate>,
    halt: &AtomicBool,
) {
    match event {
        ContentEvent::Hits {
            rel,
            size,
            mtime_ms,
            ..
        } => {
            if seen.insert(rel.clone()) {
                let candidate = Candidate {
                    rel,
                    listed: Some((size, mtime_ms)),
                    hits: None,
                };
                if todo.send(candidate).is_err() {
                    halt.store(true, Ordering::Relaxed);
                }
            }
        }
        ContentEvent::Skipped { rel, reason } => match reason {
            // Binary files hold no text to replace; Notepad++ passes them silently.
            SkipReason::Binary => {}
            SkipReason::Unreadable(cause) => run.skip(&rel, ReplaceSkip::Unreadable(cause)),
            SkipReason::UnlistableFolder(cause) => {
                run.skip(&rel, ReplaceSkip::UnlistableFolder(cause))
            }
        },
        ContentEvent::Progress { files, .. } => {
            run.searched.store(files, Ordering::Relaxed);
            run.progress();
        }
    }
}

/// The hits of a selection per file, in the order the files first appear.
fn by_file(hits: &[HitRef]) -> Vec<(String, Vec<HitRef>)> {
    let mut order = Vec::new();
    let mut groups: HashMap<&str, Vec<HitRef>> = HashMap::new();
    for hit in hits {
        let group = groups.entry(hit.rel.as_str()).or_insert_with(|| {
            order.push(hit.rel.clone());
            Vec::new()
        });
        group.push(hit.clone());
    }
    order
        .into_iter()
        .map(|rel| {
            let hits = groups.remove(rel.as_str()).unwrap_or_default();
            (rel, hits)
        })
        .collect()
}
