//! The shared state of one running content search: the matcher, the query,
//! the receiver of its events, the hit budget of `max_hits`, the totals and
//! the progress clock. Every searching thread works through one `ContentRun`;
//! a host that walks its own way (the Share host's handle-bound walk) uses it
//! with `PathFilter` and `search_reader` the same way.
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use super::content_match::ContentMatcher;
use super::content_query::{ContentEvent, ContentQuery, ContentSummary, SkipReason};

/// Shortest time between two progress events.
const PROGRESS_EVERY_MS: u64 = 250;

/// The receiver of a search's events; `false` = it is gone, the search stops.
pub type ContentEmit<'a> = &'a (dyn Fn(ContentEvent) -> bool + Sync);

/// What searching one file found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileOutcome {
    /// Hit lines of the file (context lines not counted).
    pub hits: u64,
    /// Bytes read from the file's source.
    pub bytes: u64,
    /// The file was not searched: it contains NUL bytes (binary) or, in
    /// `ContentRun::search_reader`, could not be read; reported `Skipped`.
    pub skipped: bool,
}

pub struct ContentRun<'a> {
    pub(super) matcher: &'a ContentMatcher,
    pub(super) query: &'a ContentQuery,
    emit: ContentEmit<'a>,
    cancel: &'a AtomicBool,
    hits: AtomicU64,
    files: AtomicU64,
    matched: AtomicU64,
    bytes: AtomicU64,
    skipped: AtomicU64,
    limited: AtomicBool,
    gone: AtomicBool,
    started: Instant,
    last_progress: AtomicU64,
}

impl<'a> ContentRun<'a> {
    pub fn new(
        matcher: &'a ContentMatcher,
        query: &'a ContentQuery,
        emit: ContentEmit<'a>,
        cancel: &'a AtomicBool,
    ) -> Self {
        Self {
            matcher,
            query,
            emit,
            cancel,
            hits: AtomicU64::new(0),
            files: AtomicU64::new(0),
            matched: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            limited: AtomicBool::new(false),
            gone: AtomicBool::new(false),
            started: Instant::now(),
            last_progress: AtomicU64::new(0),
        }
    }

    /// The search stops: canceled, `max_hits` reached or receiver gone.
    pub fn halted(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
            || self.limited.load(Ordering::Relaxed)
            || self.gone.load(Ordering::Relaxed)
    }

    /// The search ended at `max_hits`.
    pub fn limited(&self) -> bool {
        self.limited.load(Ordering::Relaxed)
    }

    /// `Err` when the whole search must end: canceled or receiver gone (a
    /// reached `max_hits` is a normal end).
    pub fn check(&self) -> io::Result<()> {
        if self.gone.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "content search receiver closed",
            ));
        }
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "content search canceled",
            ));
        }
        Ok(())
    }

    /// Hands `event` to the receiver; false (and the search stops) once it is gone.
    pub fn send(&self, event: ContentEvent) -> bool {
        if self.gone.load(Ordering::Relaxed) {
            return false;
        }
        let delivered = (self.emit)(event);
        if !delivered {
            self.gone.store(true, Ordering::Relaxed);
        }
        delivered
    }

    /// Reports `rel` (a file or folder) as not searched.
    pub fn skip(&self, rel: &str, reason: SkipReason) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
        self.send(ContentEvent::Skipped {
            rel: rel.to_string(),
            reason,
        });
    }

    /// One more hit line within `max_hits`; false = the budget is spent (the
    /// search ends, `limited`).
    pub(super) fn take_hit(&self) -> bool {
        let max = self.query.max_hits;
        if max == 0 {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        let taken = self
            .hits
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                (used < max).then_some(used + 1)
            });
        match taken {
            Ok(used) => {
                if used + 1 >= max {
                    self.limited.store(true, Ordering::Relaxed);
                }
                true
            }
            Err(_) => {
                self.limited.store(true, Ordering::Relaxed);
                false
            }
        }
    }

    /// Hit lines of a file that turned out binary go back to the budget.
    pub(super) fn refund(&self, hits: u64) {
        if hits > 0 {
            self.hits.fetch_sub(hits, Ordering::Relaxed);
        }
    }

    pub(super) fn count_file(&self, hits: u64) {
        self.files.fetch_add(1, Ordering::Relaxed);
        if hits > 0 {
            self.matched.fetch_add(1, Ordering::Relaxed);
        }
        self.progress();
    }

    pub(super) fn add_bytes(&self, bytes: u64) {
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
        self.progress();
    }

    /// A `Progress` event, at most every `PROGRESS_EVERY_MS`.
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
        self.send(ContentEvent::Progress {
            files: self.files.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
        });
    }

    /// The totals so far.
    pub fn summary(&self) -> ContentSummary {
        ContentSummary {
            files_searched: self.files.load(Ordering::Relaxed),
            files_matched: self.matched.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
            limited: self.limited(),
        }
    }

    /// The end of a walk that returned `walked`: the totals, or why the
    /// search failed (a walk stopped by `max_hits` is a normal end).
    pub fn finish(&self, walked: io::Result<()>) -> io::Result<ContentSummary> {
        self.check()?;
        match walked {
            Err(error) if !self.limited() => Err(error),
            _ => Ok(self.summary()),
        }
    }
}
