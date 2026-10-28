//! Copies during the scan (G4 MS14; contract FS3 as of 2026-10-28).
//!
//! FS3 now reads: deletions, and every decision that derives from absence
//! (side empty, deletion thresholds, mirror orphans, propagated deletions),
//! happen only after the complete scan of both sides. A new file may be
//! copied while the scan still runs, once the target side has listed
//! enough folders to prove the name absent there and the last complete
//! state (baseline) does not know the name, so it was never synced and
//! cannot be a deletion on the target. Updates of existing files, conflicts,
//! moves and all deletions wait for the complete plan.
//!
//! An early copy is only a head start: it publishes without replacing
//! (`ExpectedFile::Missing`), under the target's own spelling of every
//! existing folder, with the run's guards (links, mounts, own folders) and
//! durability rules. After the scan the planner still decides from both
//! complete trees; where it plans the same copy, the early result is
//! recorded as that action's completion (journal and baseline through the
//! checkpoint) and the action is not run again. A failed early copy only
//! leaves the action to the normal apply.
//!
//! The lane stays off for dry runs, moves, vault rules, providers with
//! duplicate names, recorded spelling aliases, and when a side's root is
//! empty although the pair history says it held entries (the side-empty
//! guard must decide first).
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

use super::apply_guard::ExpectedFile;
use super::incremental::SyncEndpoints;
use super::keys::KeyPolicy;
use super::snapshot_dir::Listed;
use super::snapshot_hash::{hash_mode, HashMode};
use super::types::{Baseline, BisyncOptions, Direction, PairSide, Sig, Throttle};
use super::versions::{RunVersions, VersionSide};
use crate::vfs::StageDurability;

/// Told about every folder a walk listed completely.
pub(super) trait FolderListed: Sync {
    fn listed(&self, dir_rel: &str, listed: &Listed);
}

/// One side's view of the lane, handed to that side's walk.
pub(super) struct SideListed<'l, 'a> {
    pub(super) lane: &'l EarlyLane<'a>,
    pub(super) side: PairSide,
}

impl FolderListed for SideListed<'_, '_> {
    fn listed(&self, dir_rel: &str, listed: &Listed) {
        self.lane.listed(self.side, dir_rel, listed);
    }
}

/// A file copied during the scan.
pub(super) struct EarlyCopy {
    from: PairSide,
    key: String,
    target_rel: String,
    source: Sig,
    destination: Sig,
    durable: bool,
}

struct Job {
    from: PairSide,
    rel: String,
    key: String,
    target_rel: String,
    sig: Sig,
}

/// One listed folder: its files, and every child name (files and folders)
/// by key.
struct Folder {
    rel: String,
    files: Vec<(String, Sig)>,
    names: HashSet<String>,
    colliding: HashSet<String>,
}

enum Lookup {
    Unknown,
    Present,
    Absent(String),
}

#[derive(Default)]
struct LaneState {
    folders: [BTreeMap<String, Folder>; 2],
    roots: [Option<bool>; 2],
    considered: HashSet<String>,
    queue: VecDeque<Job>,
    done: Vec<EarlyCopy>,
    stopped: bool,
}

pub(super) struct EarlyLane<'a> {
    endpoints: SyncEndpoints<'a>,
    opts: BisyncOptions,
    keys: KeyPolicy,
    base: HashSet<String>,
    held_entries: [bool; 2],
    deferred: [bool; 2],
    throttle: Throttle,
    cancel: &'a AtomicBool,
    versions: &'a RunVersions,
    state: Mutex<LaneState>,
    work: Condvar,
}

fn index(side: PairSide) -> usize {
    match side {
        PairSide::A => 0,
        PairSide::B => 1,
    }
}

fn name_of(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

impl<'a> EarlyLane<'a> {
    /// `None` where the lane stays off (see the module documentation).
    /// `held_entries`: the pair history's entry count per side was non-zero.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        endpoints: SyncEndpoints<'a>,
        opts: BisyncOptions,
        keys: KeyPolicy,
        baseline: &Baseline,
        held_entries: [bool; 2],
        aliases_recorded: bool,
        cancel: &'a AtomicBool,
        versions: &'a RunVersions,
    ) -> Option<Self> {
        let off = opts.dry_run
            || opts.move_files
            || aliases_recorded
            || super::plan_rules::current().is_some()
            || endpoints.a.has_duplicate_file_names()
            || endpoints.b.has_duplicate_file_names();
        if off {
            return None;
        }
        // The same proof as the apply's: a later real root syncfs covers a
        // deferred stage; the checkpoint performs it before recording.
        let mut deferred = [false; 2];
        if !opts.cross_mounts {
            for (slot, backend, root) in [
                (0, endpoints.a, endpoints.root_a),
                (1, endpoints.b, endpoints.root_b),
            ] {
                deferred[slot] = backend.is_local()
                    && crate::vfs::sync_filesystem(backend, root).unwrap_or(false);
            }
        }
        Some(Self {
            endpoints,
            opts,
            keys,
            base: baseline
                .keys()
                .map(|rel| keys.key(rel).into_owned())
                .collect(),
            held_entries,
            deferred,
            throttle: Throttle::new(opts.bwlimit_bps),
            cancel,
            versions,
            state: Mutex::new(LaneState::default()),
            work: Condvar::new(),
        })
    }

    /// Copy workers of the lane: as many as the wider side lists at once,
    /// never more than the job's transfer limit (`max_transfers`).
    pub(super) fn workers(&self) -> usize {
        let width = self
            .endpoints
            .a
            .parallelism()
            .max(self.endpoints.b.parallelism())
            .max(1);
        match self.opts.max_transfers {
            0 => width,
            limit => width.min(limit),
        }
    }

    fn allows(&self, from: PairSide) -> bool {
        match self.opts.direction {
            Direction::Both => true,
            Direction::AtoB => from == PairSide::A,
            Direction::BtoA => from == PairSide::B,
        }
    }

    fn side(&self, side: PairSide) -> (&'a dyn crate::vfs::Backend, &'a str) {
        match side {
            PairSide::A => (self.endpoints.a, self.endpoints.root_a),
            PairSide::B => (self.endpoints.b, self.endpoints.root_b),
        }
    }

    fn listed(&self, side: PairSide, dir_rel: &str, listed: &Listed) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.stopped {
            return;
        }
        let slot = index(side);
        if dir_rel.is_empty() {
            let empty = listed.files.is_empty() && listed.dirs.is_empty();
            if empty && self.held_entries[slot] {
                // The side-empty guard decides after the scan; nothing early.
                state.stopped = true;
                state.queue.clear();
                drop(state);
                self.work.notify_all();
                return;
            }
            state.roots[slot] = Some(empty);
        }
        let key = self.keys.key(dir_rel).into_owned();
        let folder = self.folder(dir_rel, listed);
        state.folders[slot].insert(key.clone(), folder);
        if state.roots.iter().any(Option::is_none) {
            return;
        }
        let queued = state.queue.len();
        for from in [side, side.other()] {
            // This folder and, on both sides, every folder below it: the new
            // listing may settle names there (a root listing settles all).
            let folders = &state.folders[index(from)];
            let affected = if key.is_empty() {
                folders.keys().cloned().collect::<Vec<_>>()
            } else {
                let prefix = format!("{key}/");
                folders
                    .get_key_value(&key)
                    .into_iter()
                    .chain(
                        folders
                            .range(prefix.clone()..)
                            .take_while(|(folder, _)| folder.starts_with(&prefix)),
                    )
                    .map(|(folder, _)| folder.clone())
                    .collect::<Vec<_>>()
            };
            for folder in affected {
                self.evaluate(&mut state, from, &folder);
            }
        }
        let started = state.queue.len() > queued;
        drop(state);
        if started {
            self.work.notify_all();
        }
    }

    fn folder(&self, dir_rel: &str, listed: &Listed) -> Folder {
        let mut names = HashSet::new();
        let mut colliding = HashSet::new();
        let children = listed
            .files
            .iter()
            .map(|(rel, _, _)| rel.as_str())
            .chain(listed.dirs.iter().map(|(_, rel)| rel.as_str()));
        for rel in children {
            let name = self.keys.key(name_of(rel)).into_owned();
            if !names.insert(name.clone()) {
                colliding.insert(name);
            }
        }
        Folder {
            rel: dir_rel.to_string(),
            files: listed
                .files
                .iter()
                .map(|(rel, sig, _)| (rel.clone(), *sig))
                .collect(),
            names,
            colliding,
        }
    }

    fn evaluate(&self, state: &mut LaneState, from: PairSide, folder_key: &str) {
        if !self.allows(from) {
            return;
        }
        let Some(folder) = state.folders[index(from)].get(folder_key) else {
            return;
        };
        let target = &state.folders[index(from.other())];
        let mut settled = Vec::new();
        let mut jobs = Vec::new();
        for (rel, sig) in &folder.files {
            let key = self.keys.key(rel).into_owned();
            let name = self.keys.key(name_of(rel)).into_owned();
            if state.considered.contains(&key) || folder.colliding.contains(&name) {
                continue;
            }
            match self.target_of(target, rel) {
                Lookup::Unknown => {}
                Lookup::Present => settled.push(key),
                Lookup::Absent(target_rel) => {
                    if !self.base.contains(&key) {
                        jobs.push(Job {
                            from,
                            rel: rel.clone(),
                            key: key.clone(),
                            target_rel,
                            sig: *sig,
                        });
                    }
                    settled.push(key);
                }
            }
        }
        state.considered.extend(settled);
        state.queue.extend(jobs);
    }

    /// Where `rel` would go on the side whose folders are `folders`.
    fn target_of(&self, folders: &BTreeMap<String, Folder>, rel: &str) -> Lookup {
        let parts = rel.split('/').collect::<Vec<_>>();
        let Some(mut folder) = folders.get("") else {
            return Lookup::Unknown;
        };
        let mut current = String::new();
        for (position, part) in parts.iter().enumerate() {
            let name = self.keys.key(part);
            if !folder.names.contains(name.as_ref()) {
                let mut target = folder.rel.clone();
                for rest in &parts[position..] {
                    if !target.is_empty() {
                        target.push('/');
                    }
                    target.push_str(rest);
                }
                return Lookup::Absent(target);
            }
            if position + 1 == parts.len() {
                return Lookup::Present;
            }
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(&name);
            match folders.get(&current) {
                Some(next) => folder = next,
                None => return Lookup::Unknown,
            }
        }
        Lookup::Unknown
    }

    /// Runs early copies until `stop`.
    pub(super) fn work(&self) {
        loop {
            let job = {
                let Ok(mut state) = self.state.lock() else {
                    return;
                };
                loop {
                    if state.stopped || self.cancel.load(Ordering::Acquire) {
                        return;
                    }
                    if let Some(job) = state.queue.pop_front() {
                        break job;
                    }
                    state = match self.work.wait(state) {
                        Ok(state) => state,
                        Err(_) => return,
                    };
                }
            };
            if let Some(done) = self.copy(&job) {
                if let Ok(mut state) = self.state.lock() {
                    state.done.push(done);
                }
            }
        }
    }

    fn copy(&self, job: &Job) -> Option<EarlyCopy> {
        let to = job.from.other();
        let (source, source_root) = self.side(job.from);
        let (target, target_root) = self.side(to);
        let sp = crate::vfs::sync_path(source, source_root, &job.rel).ok()?;
        let dp = crate::vfs::sync_path(target, target_root, &job.target_rel).ok()?;
        let durability = if self.deferred[index(to)] {
            StageDurability::Deferred
        } else {
            StageDurability::Now
        };
        let outcome = super::apply_transaction::copy(
            source,
            &sp,
            ExpectedFile::Present(job.sig),
            &VersionSide {
                side: to,
                backend: target,
                root: target_root,
            },
            &dp,
            ExpectedFile::Missing,
            &job.target_rel,
            self.opts,
            Some(self.versions),
            durability,
            &self.throttle,
            self.cancel,
            None,
            |_| {},
            |_| {},
        )
        .ok()?;
        if !outcome.durable && durability != StageDurability::Deferred {
            // Published but unconfirmed: the normal apply reports it.
            return None;
        }
        Some(EarlyCopy {
            from: job.from,
            key: job.key.clone(),
            target_rel: job.target_rel.clone(),
            source: self.hashed(job.from, outcome.source),
            destination: self.hashed(to, outcome.destination),
            durable: outcome.durable,
        })
    }

    /// Hashes follow each side's hash mode, as in the apply.
    fn hashed(&self, side: PairSide, mut sig: Sig) -> Sig {
        let (this, _) = self.side(side);
        let (other, _) = self.side(side.other());
        if matches!(hash_mode(this, other, self.opts.compare), HashMode::None) {
            sig.hash = 0;
        }
        sig
    }

    /// No new early copies; running ones finish.
    pub(super) fn stop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stopped = true;
            state.queue.clear();
        }
        self.work.notify_all();
    }

    pub(super) fn into_done(self) -> Vec<EarlyCopy> {
        self.state
            .into_inner()
            .map(|state| state.done)
            .unwrap_or_default()
    }
}

#[path = "early_copy_record.rs"]
mod record;
pub(super) use record::{read_pair_copying, record};

#[cfg(test)]
#[path = "early_copy_tests.rs"]
mod tests;
