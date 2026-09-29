//! Explorer's copies in the app's transfer list: one entry per paste (or
//! drop) with the listed files, delivered files and bytes, the entries
//! Explorer cannot receive, and errors.
use super::catalog::{Catalog, TOO_LARGE_NOTE};
use crate::transfer::{register_external, ExternalTransfer};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

/// "sucht… N gefunden" is refreshed at most every 250 ms: four updates a
/// second read as continuous progress, and hundreds of thousands of listed
/// entries do not each take the entry's lock.
const FOUND_NOTE_MS: u64 = 250;
const SEARCHING_NOTE: &str = "sucht…";
const COPYING_NOTE: &str = "Explorer kopiert";
const DONE_NOTE: &str = "vom Explorer übernommen";
const ENDED_NOTE: &str = "Explorer-Übergabe beendet";

/// What the listing found; applies to every paste of the object.
struct Summary {
    files: u64,
    errors: u64,
    note: Option<String>,
    /// The omitted entries with their reasons (the first ones; `errors`
    /// counts all of them).
    issues: Vec<(String, String)>,
}

struct Session {
    progress: Arc<ExternalTransfer>,
    delivered: HashSet<usize>,
    failed: HashSet<usize>,
    /// Streams Explorer holds open now.
    streams: usize,
    /// Explorer opened a stream in this paste.
    read: bool,
    errors: u64,
    too_large: bool,
}

impl Session {
    fn start(label: &str, summary: Option<&Summary>) -> Self {
        let progress = register_external(label.to_string());
        let mut session = Self {
            progress,
            delivered: HashSet::new(),
            failed: HashSet::new(),
            streams: 0,
            read: false,
            errors: 0,
            too_large: false,
        };
        match summary {
            Some(summary) => session.apply(summary),
            None => session.progress.set_note(SEARCHING_NOTE),
        }
        session
    }

    fn apply(&mut self, summary: &Summary) {
        self.progress.set_files_total(summary.files);
        for (path, message) in &summary.issues {
            self.progress.issue(path.as_str(), message.as_str());
        }
        for _ in summary.issues.len() as u64..summary.errors {
            self.progress.error();
        }
        self.errors += summary.errors;
        let note = summary.note.as_deref().unwrap_or(COPYING_NOTE);
        self.progress.set_note(note);
    }
}

#[derive(Default)]
struct Slot {
    summary: Option<Summary>,
    active: Option<Session>,
    /// Finished entries stay listed while the hand-off lives.
    finished: Vec<Arc<ExternalTransfer>>,
}

impl Slot {
    fn session(&mut self, label: &str) -> &mut Session {
        let summary = self.summary.as_ref();
        self.active
            .get_or_insert_with(|| Session::start(label, summary))
    }

    /// Ends the current entry; one where nothing was read and nothing went
    /// wrong (a cancelled drag) just disappears.
    fn retire(&mut self) {
        let Some(session) = self.active.take() else {
            return;
        };
        if !session.read && session.errors == 0 {
            return;
        }
        if session.errors == 0 {
            let files = self.summary.as_ref().map_or(0, |summary| summary.files);
            let complete = session.delivered.len() as u64 >= files;
            session
                .progress
                .set_note(if complete { DONE_NOTE } else { ENDED_NOTE });
        }
        session.progress.finish();
        self.finished.push(session.progress);
    }
}

pub(super) struct Sessions {
    label: String,
    epoch: Instant,
    last_found_note_ms: AtomicU64,
    slot: Mutex<Slot>,
}

impl Sessions {
    pub(super) fn new(label: String) -> Self {
        Self {
            label,
            epoch: Instant::now(),
            last_found_note_ms: AtomicU64::new(0),
            slot: Mutex::new(Slot::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Slot> {
        self.slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Explorer asks for the file list: a new paste once the current one has
    /// read files and closed them all (Explorer asks more than once per
    /// paste before reading). True when a new paste began.
    pub(super) fn begin(&self) -> bool {
        let mut slot = self.lock();
        if matches!(&slot.active, Some(session) if session.streams > 0 || !session.read) {
            return false;
        }
        slot.retire();
        slot.session(&self.label);
        true
    }

    /// Listing progress while Explorer waits for the list.
    pub(super) fn found(&self, count: u64) {
        let now = self.epoch.elapsed().as_millis() as u64;
        let last = self.last_found_note_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) < FOUND_NOTE_MS
            || self
                .last_found_note_ms
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return;
        }
        let slot = self.lock();
        if let (None, Some(session)) = (&slot.summary, &slot.active) {
            session
                .progress
                .set_note(format!("{SEARCHING_NOTE} {count} gefunden"));
        }
    }

    /// The listing finished: totals and omissions for this and later pastes.
    pub(super) fn listed(&self, catalog: &Catalog) {
        let summary = Summary {
            files: catalog.files,
            errors: catalog.errors(),
            note: catalog.note(),
            issues: catalog.issues(),
        };
        let mut slot = self.lock();
        if let Some(session) = slot.active.as_mut() {
            session.apply(&summary);
        }
        slot.summary = Some(summary);
    }

    pub(super) fn stream_opened(&self) {
        let mut slot = self.lock();
        let session = slot.session(&self.label);
        session.streams += 1;
        session.read = true;
    }

    pub(super) fn stream_closed(&self) {
        if let Some(session) = self.lock().active.as_mut() {
            session.streams = session.streams.saturating_sub(1);
        }
    }

    pub(super) fn bytes(&self, count: u64) {
        if let Some(session) = self.lock().active.as_ref() {
            session.progress.add_bytes(count);
        }
    }

    /// Explorer read file `index` to its end; the paste is complete once
    /// every listed file arrived.
    pub(super) fn delivered(&self, index: usize) {
        let mut slot = self.lock();
        let files = slot
            .summary
            .as_ref()
            .map_or(u64::MAX, |summary| summary.files);
        let Some(session) = slot.active.as_mut() else {
            return;
        };
        if !session.delivered.insert(index) {
            return;
        }
        session.progress.file_done();
        if session.delivered.len() as u64 >= files {
            slot.retire();
        }
    }

    /// Reading file `index` failed; counted once per file and paste.
    pub(super) fn failed(&self, index: usize, rel: &str, message: &str) {
        let mut slot = self.lock();
        let session = slot.session(&self.label);
        session.progress.set_note(format!("{rel}: {message}"));
        if session.failed.insert(index) {
            session.errors += 1;
            session.progress.issue(rel, message);
        }
    }

    /// Windows could not take the file list (K19); counted once per paste.
    pub(super) fn too_large(&self) {
        let mut slot = self.lock();
        let session = slot.session(&self.label);
        session.progress.set_note(TOO_LARGE_NOTE);
        if !session.too_large {
            session.too_large = true;
            session.errors += 1;
            session.progress.issue(String::new(), TOO_LARGE_NOTE);
        }
    }

    /// Explorer finished its background copy, or the hand-off is over.
    pub(super) fn end(&self) {
        self.lock().retire();
    }
}
