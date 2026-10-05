//! Runtime checkpoint sink: bounded pending batches, a periodic heartbeat,
//! target flush before journaling, and final compaction even after cancel.
use std::io;
use std::sync::{mpsc, Mutex, MutexGuard};
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

const CHECKPOINT_ACTIONS: usize = 64;
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(2);

struct State {
    records: RecordBook,
    dirs: DirSet,
    dirs_bytes: u64,
    journal: Journal,
    pending: Vec<CompletedAction>,
    omitted: Vec<(String, OmissionKind)>,
    deferred: Vec<(String, String)>,
    stopped: Option<RunStop>,
    error: Option<String>,
    last: Instant,
    counts: Option<[u64; 2]>,
    presence: std::collections::BTreeMap<(PairSide, String, bool), bool>,
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
    endpoints: SyncEndpoints<'a>,
    lock: &'a PairLock,
    key: &'a StateKey,
    keys: KeyPolicy,
    observer: Option<&'a dyn ApplySink>,
    observed: Option<([&'a SideSnapshot; 2], &'a Spellings)>,
    aliases: Option<&'a PathAliases>,
    state: Mutex<State>,
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
                omitted: Vec::new(),
                deferred: Vec::new(),
                stopped: None,
                error: None,
                last: Instant::now(),
                counts: None,
                presence: Default::default(),
            }),
        })
    }

    pub fn with_observations(
        mut self,
        observed: [&'a SideSnapshot; 2],
        spellings: &'a Spellings,
        counts: [u64; 2],
    ) -> Self {
        self.observed = Some((observed, spellings));
        self.lock().counts = Some(counts);
        self
    }

    pub fn with_path_aliases(mut self, aliases: &'a PathAliases) -> Self {
        self.aliases = Some(aliases);
        self
    }

    fn directory_key(&self, rel: &str, side: PairSide) -> String {
        self.aliases.map_or_else(
            || self.keys.key(rel).into_owned(),
            |aliases| aliases.key(rel, side, self.keys),
        )
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    /// The timer flushes completed work even while the next transfer is slow.
    /// It is joined before returning, and does not change the user's cancel.
    pub fn during<T>(&self, work: impl FnOnce() -> T) -> T {
        std::thread::scope(|scope| {
            let (end, wait) = mpsc::channel::<()>();
            let timer = std::thread::Builder::new()
                .name("sync-checkpoint".into())
                .spawn_scoped(scope, move || {
                    while matches!(
                        wait.recv_timeout(CHECKPOINT_INTERVAL),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        let mut state = self.lock();
                        self.flush(&mut state);
                    }
                });
            if let Err(error) = &timer {
                self.lock().error.get_or_insert_with(|| {
                    format!("Zwischenstand kann nicht gestartet werden: {error}")
                });
            }
            let result = work();
            let _ = end.send(());
            if let Ok(timer) = timer {
                if timer.join().is_err() {
                    self.lock()
                        .error
                        .get_or_insert_with(|| "Zwischenstand unerwartet beendet".into());
                }
            }
            result
        })
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
        let state = self
            .state
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        CheckpointResult {
            baseline: state.records.baseline,
            dirs: state.dirs,
            omitted: state.omitted,
            deferred: state.deferred,
            stopped: state.stopped,
            error: state.error,
            counts: state.counts,
        }
    }

    /// Rename cleanup must wait for the upserts, including their deferrals
    /// and omissions; neither is an ordinary error in ApplyReport.
    pub fn can_delete(&self) -> bool {
        let mut state = self.lock();
        self.flush(&mut state);
        state.error.is_none()
            && state.stopped.is_none()
            && state.pending.is_empty()
            && state.omitted.is_empty()
            && state.deferred.is_empty()
    }

    pub fn completed_deletion(&self, rel: &str, side: PairSide) -> bool {
        self.lock()
            .presence
            .get(&(side, self.keys.key(rel).into_owned(), false))
            == Some(&false)
    }

    pub fn protected_paths(&self) -> super::SyncOmissions {
        let state = self.lock();
        let mut protected = super::SyncOmissions::new(self.keys.fold_case);
        for (rel, kind) in &state.omitted {
            protected.record_kind(rel, *kind, kind.reported_by_default());
        }
        for (rel, _) in &state.deferred {
            protected.record_kind(rel, OmissionKind::Unreadable, false);
        }
        protected
    }

    fn flush(&self, state: &mut State) {
        if state.pending.is_empty() {
            return;
        }
        if self.lock.id() != self.key.lock_id.to_ascii_lowercase() {
            state
                .error
                .get_or_insert_with(|| "Zwischenstand hält die falsche Paarsperre".into());
            return;
        }
        let mut flush_a = false;
        let mut flush_b = false;
        for result in state.pending.iter().filter(|action| !action.durable) {
            for side in durability_sides(result.kind) {
                match side {
                    PairSide::A => flush_a = true,
                    PairSide::B => flush_b = true,
                }
            }
        }
        let durable_a = self.flush_side(PairSide::A, flush_a, state);
        let durable_b = self.flush_side(PairSide::B, flush_b, state);
        let mut frame = Frame {
            fold_case: self.keys.fold_case,
            ..Frame::default()
        };
        let mut waiting = Vec::new();
        for action in std::mem::take(&mut state.pending) {
            if !action.durable
                && !durability_sides(action.kind)
                    .into_iter()
                    .all(|side| match side {
                        PairSide::A => durable_a,
                        PairSide::B => durable_b,
                    })
            {
                waiting.push(action);
                continue;
            }
            if let Some(entry) = action.baseline_entry() {
                frame.records.push((action.rel.clone(), entry));
            } else {
                match action.kind {
                    CompletedKind::DirCreated { side } => {
                        frame.dirs_add.push(self.directory_key(&action.rel, side));
                    }
                    CompletedKind::DirRemoved { side } => {
                        frame
                            .dirs_remove
                            .push(self.directory_key(&action.rel, side));
                    }
                    _ => {}
                }
            }
        }
        if !frame.is_empty() {
            let result = frame
                .ensure_fits(
                    &state.records,
                    &state.dirs,
                    state.dirs_bytes,
                    super::SyncLimits::for_memory(crate::transfer::physical_memory()),
                )
                .and_then(|bytes| {
                    state.journal.append(&frame)?;
                    frame.apply(&mut state.records, &mut state.dirs)?;
                    state.dirs_bytes = bytes;
                    Ok(())
                });
            if let Err(error) = result {
                state.error.get_or_insert_with(|| error.to_string());
                // Keep authoritative deltas for another final flush attempt.
                for (rel, entry) in frame.records {
                    waiting.push(CompletedAction {
                        rel,
                        kind: CompletedKind::Copied { from: PairSide::A },
                        src_sig: entry.0,
                        dst_sig: entry.1,
                        durable: true,
                    });
                }
                for rel in frame.dirs_add {
                    waiting.push(CompletedAction {
                        rel,
                        kind: CompletedKind::DirCreated { side: PairSide::A },
                        src_sig: None,
                        dst_sig: None,
                        durable: true,
                    });
                }
                for rel in frame.dirs_remove {
                    waiting.push(CompletedAction {
                        rel,
                        kind: CompletedKind::DirRemoved { side: PairSide::A },
                        src_sig: None,
                        dst_sig: None,
                        durable: true,
                    });
                }
            }
        }
        state.pending = waiting;
        state.last = Instant::now();
    }

    fn flush_side(&self, side: PairSide, needed: bool, state: &mut State) -> bool {
        if !needed {
            return true;
        }
        let (backend, root) = match side {
            PairSide::A => (self.endpoints.a, self.endpoints.root_a),
            PairSide::B => (self.endpoints.b, self.endpoints.root_b),
        };
        match crate::vfs::sync_filesystem(backend, root) {
            Ok(true) => true,
            Ok(false) => {
                state.error.get_or_insert_with(|| {
                    format!(
                        "Dateien auf {} noch nicht dauerhaft bestätigt",
                        side.label()
                    )
                });
                false
            }
            Err(error) => {
                state.error.get_or_insert_with(|| error.to_string());
                false
            }
        }
    }

    fn count_completed(&self, state: &mut State, action: &CompletedAction) {
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
            let was_present = state
                .presence
                .insert((side, key, directory), present)
                .unwrap_or(initial);
            if let Some(counts) = &mut state.counts {
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

impl ApplySink for CheckpointSink<'_> {
    fn completed(&self, action: CompletedAction) {
        let mut state = self.lock();
        self.count_completed(&mut state, &action);
        state.pending.push(action.clone());
        if state.pending.len() >= CHECKPOINT_ACTIONS || state.last.elapsed() >= CHECKPOINT_INTERVAL
        {
            self.flush(&mut state);
        }
        drop(state);
        if let Some(observer) = self.observer {
            observer.completed(action);
        }
    }

    fn should_stop(&self) -> bool {
        let state = self.lock();
        let stopped = state.error.is_some() || state.stopped.is_some();
        drop(state);
        stopped || self.observer.is_some_and(|observer| observer.should_stop())
    }

    fn omitted(&self, rel: &str, kind: OmissionKind) {
        self.lock().omitted.push((rel.to_string(), kind));
        if let Some(observer) = self.observer {
            observer.omitted(rel, kind);
        }
    }

    fn deferred(&self, rel: &str, reason: &str) {
        self.lock()
            .deferred
            .push((rel.to_string(), reason.to_string()));
        if let Some(observer) = self.observer {
            observer.deferred(rel, reason);
        }
    }

    fn stopped(&self, stop: RunStop) {
        self.lock().stopped.get_or_insert(stop);
        if let Some(observer) = self.observer {
            observer.stopped(stop);
        }
    }
}

fn durability_sides(kind: CompletedKind) -> Vec<PairSide> {
    match kind {
        CompletedKind::Copied { from } => vec![from.other()],
        CompletedKind::Moved { from } => vec![from, from.other()],
        CompletedKind::Deleted { side }
        | CompletedKind::DirCreated { side }
        | CompletedKind::DirRemoved { side } => vec![side],
    }
}
