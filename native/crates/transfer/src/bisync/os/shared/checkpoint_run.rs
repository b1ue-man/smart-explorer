//! Runtime checkpoint sink: bounded pending batches, a group committer,
//! target flush before journaling, and final compaction even after cancel.
//!
//! Workers only queue their results (`Inbox`, a short lock) and read the
//! stop state from an atomic flag; while `during` runs, the committer thread
//! `sync-checkpoint` flushes the queue as one group (target `syncfs`, then
//! one journal frame), so no worker waits for a flush (G4 MS5). Without a
//! running committer, or when the queue outgrows its bound, the reporting
//! thread flushes itself as before.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::baseline_records::RecordBook;
use super::checkpoint_journal::{Frame, Journal};
use super::completion::{ApplySink, CompletedAction, CompletedKind};
use super::incremental::SyncEndpoints;
use super::keys::{KeyPolicy, PathAliases, Spellings};
use super::omissions::OmissionKind;
use super::pair_lock::PairLock;
use super::run_types::{RunStop, StateKey};
use super::snapshot_types::{DirSet, SideSnapshot};
use super::types::{Baseline, PairSide};

/// Without a committer: the reporting thread flushes after this many
/// actions or this much time (the established batch).
pub(super) const CHECKPOINT_ACTIONS: usize = 64;
pub(super) const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(2);
/// With a committer: one group commit per this many queued actions, or
/// after this much time with anything queued. A shorter interval only
/// loses less finished work to an interruption; no worker waits for it.
pub(super) const GROUP_ACTIONS: usize = 1024;
pub(super) const GROUP_INTERVAL: Duration = Duration::from_millis(250);
/// The queue bound: past it a reporting worker commits itself, so memory
/// stays bounded when the committer falls behind a slow target flush.
pub(super) const INLINE_ACTIONS: usize = 8 * GROUP_ACTIONS;

/// Commit state: owned by whoever flushes.
pub(super) struct State {
    pub(super) records: RecordBook,
    pub(super) dirs: DirSet,
    pub(super) dirs_bytes: u64,
    pub(super) journal: Journal,
    /// Actions an earlier flush could not record yet (target not durable).
    pub(super) pending: Vec<CompletedAction>,
    pub(super) error: Option<String>,
    pub(super) last: Instant,
}

/// What reporting workers hand over; only ever held briefly.
#[derive(Default)]
pub(super) struct Inbox {
    pub(super) queued: Vec<CompletedAction>,
    omitted: Vec<(String, OmissionKind)>,
    deferred: Vec<(String, String)>,
    stopped: Option<RunStop>,
    counts: Option<[u64; 2]>,
    presence: std::collections::BTreeMap<(PairSide, String, bool), bool>,
    /// A committer thread is running (`during`).
    committer: bool,
    /// `during`'s work returned; the committer ends.
    ending: bool,
}

pub(super) struct CheckpointResult {
    pub baseline: Baseline,
    pub dirs: DirSet,
    pub omitted: Vec<(String, OmissionKind)>,
    pub deferred: Vec<(String, String)>,
    pub stopped: Option<RunStop>,
    pub error: Option<String>,
    pub counts: Option<[u64; 2]>,
}

pub(super) struct CheckpointSink<'a> {
    pub(super) endpoints: SyncEndpoints<'a>,
    pub(super) lock: &'a PairLock,
    pub(super) key: &'a StateKey,
    pub(super) keys: KeyPolicy,
    observer: Option<&'a dyn ApplySink>,
    observed: Option<([&'a SideSnapshot; 2], &'a Spellings)>,
    pub(super) aliases: Option<&'a PathAliases>,
    pub(super) state: Mutex<State>,
    pub(super) inbox: Mutex<Inbox>,
    wake: Condvar,
    /// An error or stop was recorded: no new admission (read lock-free).
    pub(super) halted: AtomicBool,
}

impl<'a> CheckpointSink<'a> {
    pub fn new(
        endpoints: SyncEndpoints<'a>,
        lock: &'a PairLock,
        key: &'a StateKey,
        keys: KeyPolicy,
        observer: Option<&'a dyn ApplySink>,
    ) -> io::Result<Self> {
        let (journal, records, dirs) = Journal::load(key, keys)?;
        let mut journal = journal;
        // Check storage before the first mutation, including an otherwise
        // empty plan with no convergence records.
        journal.append(&Frame {
            fold_case: keys.fold_case,
            ..Frame::default()
        })?;
        let dirs = dirs.unwrap_or_default();
        let dirs_bytes = super::checkpoint_journal::dir_bytes(&dirs);
        Ok(Self {
            endpoints,
            lock,
            key,
            keys,
            observer,
            observed: None,
            aliases: None,
            state: Mutex::new(State {
                records,
                dirs,
                dirs_bytes,
                journal,
                pending: Vec::new(),
                error: None,
                last: Instant::now(),
            }),
            inbox: Mutex::new(Inbox::default()),
            wake: Condvar::new(),
            halted: AtomicBool::new(false),
        })
    }

    pub fn with_observations(
        mut self,
        observed: [&'a SideSnapshot; 2],
        spellings: &'a Spellings,
        counts: [u64; 2],
    ) -> Self {
        self.observed = Some((observed, spellings));
        self.inbox().counts = Some(counts);
        self
    }

    pub fn with_path_aliases(mut self, aliases: &'a PathAliases) -> Self {
        self.aliases = Some(aliases);
        self
    }

    pub(super) fn directory_key(&self, rel: &str, side: PairSide) -> String {
        self.aliases.map_or_else(
            || self.keys.key(rel).into_owned(),
            |aliases| aliases.key(rel, side, self.keys),
        )
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn inbox(&self) -> MutexGuard<'_, Inbox> {
        self.inbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records a commit error and stops admission.
    pub(super) fn fail(&self, state: &mut State, error: String) {
        state.error.get_or_insert(error);
        self.halted.store(true, Ordering::Release);
    }

    /// Already observed convergence and stale spellings, without file I/O.
    pub fn planned(&self, mut frame: Frame) -> io::Result<()> {
        frame.fold_case = self.keys.fold_case;
        for rel in &mut frame.dirs_add {
            *rel = self.keys.key(rel).into_owned();
        }
        for rel in &mut frame.dirs_remove {
            *rel = self.keys.key(rel).into_owned();
        }
        let mut state = self.lock();
        if !frame.is_empty() {
            let bytes = frame.ensure_fits(
                &state.records,
                &state.dirs,
                state.dirs_bytes,
                super::SyncLimits::for_memory(crate::transfer::physical_memory()),
            )?;
            state.journal.append(&frame)?;
            let State { records, dirs, .. } = &mut *state;
            frame.apply(records, dirs)?;
            state.dirs_bytes = bytes;
        }
        Ok(())
    }

    /// The committer records completed work as group commits, also while the
    /// next transfer is slow. It is joined before returning, and does not
    /// change the user's cancel.
    pub fn during<T>(&self, work: impl FnOnce() -> T) -> T {
        std::thread::scope(|scope| {
            {
                let mut inbox = self.inbox();
                inbox.committer = true;
                inbox.ending = false;
            }
            let committer = std::thread::Builder::new()
                .name("sync-checkpoint".into())
                .spawn_scoped(scope, || self.commit_loop());
            if let Err(error) = &committer {
                self.inbox().committer = false;
                let mut state = self.lock();
                self.fail(
                    &mut state,
                    format!("Zwischenstand kann nicht gestartet werden: {error}"),
                );
            }
            // Ends the committer also when `work` unwinds, so the scope can
            // join it.
            let end = EndCommitter(self);
            let result = work();
            drop(end);
            if let Ok(committer) = committer {
                if committer.join().is_err() {
                    let mut state = self.lock();
                    self.fail(&mut state, "Zwischenstand unerwartet beendet".into());
                }
            }
            result
        })
    }

    fn commit_loop(&self) {
        loop {
            let inbox = self.inbox();
            let (inbox, _) = self
                .wake
                .wait_timeout_while(inbox, GROUP_INTERVAL, |inbox| {
                    !inbox.ending && inbox.queued.len() < GROUP_ACTIONS
                })
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if inbox.ending {
                return;
            }
            drop(inbox);
            let mut state = self.lock();
            self.flush(&mut state);
        }
    }

    pub fn finish(self) -> CheckpointResult {
        {
            let mut state = self.lock();
            self.flush(&mut state);
            if state.pending.is_empty() {
                let State {
                    journal,
                    records,
                    dirs,
                    error,
                    ..
                } = &mut *state;
                if let Err(failed) = journal.compact(self.key, records, dirs) {
                    error.get_or_insert_with(|| failed.to_string());
                }
            }
        }
        let inbox = self
            .inbox
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self
            .state
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        CheckpointResult {
            baseline: state.records.baseline,
            dirs: state.dirs,
            omitted: inbox.omitted,
            deferred: inbox.deferred,
            stopped: inbox.stopped,
            error: state.error,
            counts: inbox.counts,
        }
    }

    /// Rename cleanup must wait for the upserts, including their deferrals
    /// and omissions; neither is an ordinary error in ApplyReport.
    pub fn can_delete(&self) -> bool {
        let mut state = self.lock();
        self.flush(&mut state);
        let inbox = self.inbox();
        state.error.is_none()
            && inbox.stopped.is_none()
            && state.pending.is_empty()
            && inbox.queued.is_empty()
            && inbox.omitted.is_empty()
            && inbox.deferred.is_empty()
    }

    pub fn completed_deletion(&self, rel: &str, side: PairSide) -> bool {
        self.inbox()
            .presence
            .get(&(side, self.keys.key(rel).into_owned(), false))
            == Some(&false)
    }

    pub fn protected_paths(&self) -> super::SyncOmissions {
        let inbox = self.inbox();
        let mut protected = super::SyncOmissions::new(self.keys.fold_case);
        for (rel, kind) in &inbox.omitted {
            protected.record_kind(rel, *kind, kind.reported_by_default());
        }
        for (rel, _) in &inbox.deferred {
            protected.record_kind(rel, OmissionKind::Unreadable, false);
        }
        protected
    }

    fn count_completed(&self, inbox: &mut Inbox, action: &CompletedAction) {
        let Some((observed, spellings)) = self.observed else {
            return;
        };
        let transitions = match action.kind {
            CompletedKind::Copied { from } => vec![(from.other(), false, true)],
            CompletedKind::Moved { from } => {
                vec![(from, false, false), (from.other(), false, true)]
            }
            CompletedKind::Deleted { side } => vec![(side, false, false)],
            CompletedKind::DirCreated { side } => vec![(side, true, true)],
            CompletedKind::DirRemoved { side } => vec![(side, true, false)],
        };
        for (side, directory, present) in transitions {
            let index = if side == PairSide::A { 0 } else { 1 };
            let key = self.keys.key(&action.rel).into_owned();
            let initial = if directory {
                // Directory rels are side spellings, as in DirAction.
                observed[index].dirs.contains(&action.rel)
            } else {
                // A planner action's rel may be the other side's spelling.
                let literal = spellings.side_rel(&action.rel, side);
                observed[index].tree.contains_key(literal)
                    || observed[index].filtered.contains_key(literal)
            };
            let was_present = inbox
                .presence
                .insert((side, key, directory), present)
                .unwrap_or(initial);
            if let Some(counts) = &mut inbox.counts {
                if present && !was_present {
                    counts[index] = counts[index].saturating_add(1);
                }
                if !present && was_present {
                    counts[index] = counts[index].saturating_sub(1);
                }
            }
        }
    }
}

/// Tells a running committer to end (see `during`).
struct EndCommitter<'s, 'a>(&'s CheckpointSink<'a>);

impl Drop for EndCommitter<'_, '_> {
    fn drop(&mut self) {
        {
            let mut inbox = self.0.inbox();
            inbox.committer = false;
            inbox.ending = true;
        }
        self.0.wake.notify_all();
    }
}

impl ApplySink for CheckpointSink<'_> {
    fn completed(&self, action: CompletedAction) {
        let (queued, committer) = {
            let mut inbox = self.inbox();
            self.count_completed(&mut inbox, &action);
            inbox.queued.push(action.clone());
            (inbox.queued.len(), inbox.committer)
        };
        if committer {
            if queued >= GROUP_ACTIONS {
                self.wake.notify_all();
            }
            if queued >= INLINE_ACTIONS {
                let mut state = self.lock();
                self.flush(&mut state);
            }
        } else {
            let mut state = self.lock();
            if queued >= CHECKPOINT_ACTIONS || state.last.elapsed() >= CHECKPOINT_INTERVAL {
                self.flush(&mut state);
            }
        }
        if let Some(observer) = self.observer {
            observer.completed(action);
        }
    }

    fn should_stop(&self) -> bool {
        self.halted.load(Ordering::Acquire)
            || self.observer.is_some_and(|observer| observer.should_stop())
    }

    fn omitted(&self, rel: &str, kind: OmissionKind) {
        self.inbox().omitted.push((rel.to_string(), kind));
        if let Some(observer) = self.observer {
            observer.omitted(rel, kind);
        }
    }

    fn deferred(&self, rel: &str, reason: &str) {
        self.inbox()
            .deferred
            .push((rel.to_string(), reason.to_string()));
        if let Some(observer) = self.observer {
            observer.deferred(rel, reason);
        }
    }

    fn stopped(&self, stop: RunStop) {
        self.inbox().stopped.get_or_insert(stop);
        self.halted.store(true, Ordering::Release);
        if let Some(observer) = self.observer {
            observer.stopped(stop);
        }
    }
}

#[cfg(test)]
#[path = "checkpoint_commit_tests.rs"]
mod commit_tests;
