//! Folder keys of both sides of a pair, compared before any entry crosses a
//! connection (E10, first comparison without stored keys). Phase 1: every
//! remote side forms its keys where its files are and sends only the keys
//! (`KeyedWalk::KeysOnly`); a local side forms its keys and runs with its
//! one snapshot walk (`snapshot_local_keys.rs`). Both sides do this at the
//! same time.
//! The plan then names, per remote side, the folders whose items nothing on
//! this device holds yet: not stored for either side, not on the local side,
//! not fetched from the other side (each distinct key once, from the side
//! with fewer folders to fetch). Phase 2: each remote side sends the runs of
//! its named folders (`KeyedWalk::Runs`), again both at once. Each side's
//! snapshot is then replayed from these records (`snapshot_pair_side.rs`),
//! so it is the snapshot of the walk without keys.
//!
//! A side that does not take part (no keyed walk, a hash mode without host
//! walk) declines; a side that fails stops the pair through the shared stop
//! flag, which every wait watches.
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use super::folder_key_cache::FolderCache;
use crate::agent_proto::FolderKey;

/// How often a wait looks at the stop flag.
const WAIT_SLICE: Duration = Duration::from_millis(50);

/// The keys of one side after phase 1.
#[derive(Default)]
pub(super) struct SideTree {
    /// Every folder's key, by path below the root (`""` the root).
    pub(super) keys: HashMap<String, FolderKey>,
    /// The runs a local side read (complete for its tree); empty remotely.
    pub(super) records: Arc<FolderCache>,
}

/// The setup of one side: a remote side whose hash mode its host walk serves
/// takes part with the keys it stored; a local side as well (no keys stored).
pub(super) fn side_setup(
    backend: &dyn crate::vfs::Backend,
    root: &str,
    hash: super::snapshot_hash::HashMode,
) -> SideSetup {
    use super::snapshot_hash::HashMode;
    let eligible =
        matches!(hash, HashMode::None | HashMode::FullFresh) && !backend.has_duplicate_file_names();
    let remote = !backend.is_local();
    let stored = if remote && eligible {
        let algorithm = (hash == HashMode::FullFresh).then_some("md5");
        let files = super::snapshot_keyed::KeyFiles::of(backend, root, None, algorithm);
        super::folder_key_file::load(&files.own)
    } else {
        FolderCache::default()
    };
    SideSetup {
        remote,
        eligible,
        stored,
    }
}

/// What one side is in the pair.
pub(super) struct SideSetup {
    pub(super) remote: bool,
    /// Its walk can carry folder keys (hash mode and backend allow it).
    pub(super) eligible: bool,
    /// The keys it stored after its last walk (empty locally).
    pub(super) stored: FolderCache,
}

#[derive(Default)]
struct State {
    keys: [Option<Option<Arc<SideTree>>>; 2],
    plan: Option<[Vec<String>; 2]>,
    fetched: [Option<Arc<FolderCache>>; 2],
}

pub(super) struct PairKeys {
    takes_part: [bool; 2],
    stored: [Arc<FolderCache>; 2],
    state: Mutex<State>,
    changed: Condvar,
}

impl PairKeys {
    /// The pair's key phases, or `None` when no side needs them: every
    /// remote side that can carry keys has stored keys (its walk with known
    /// keys then sends only marks for what is unchanged).
    pub(super) fn new(sides: [SideSetup; 2]) -> Option<Self> {
        let remote_eligible = [0, 1].map(|i| sides[i].remote && sides[i].eligible);
        let takes_part =
            [0, 1].map(|i| sides[i].eligible && (sides[i].remote || remote_eligible[1 - i]));
        let needed = (0..2).any(|i| remote_eligible[i] && sides[i].stored.len() == 0);
        if !needed || !takes_part.iter().any(|t| *t) {
            return None;
        }
        let [a, b] = sides;
        let mut state = State::default();
        for (side, takes) in takes_part.iter().enumerate() {
            if !takes {
                state.keys[side] = Some(None);
                state.fetched[side] = Some(Arc::new(FolderCache::default()));
            }
        }
        Some(Self {
            takes_part,
            stored: [Arc::new(a.stored), Arc::new(b.stored)],
            state: Mutex::new(state),
            changed: Condvar::new(),
        })
    }

    pub(super) fn takes_part(&self, side: usize) -> bool {
        self.takes_part[side]
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Phase 1 of `side`: its keys, or `None` when it does not take part
    /// after all. The first submission counts.
    pub(super) fn submit_keys(&self, side: usize, tree: Option<SideTree>) {
        let mut state = self.lock();
        if state.keys[side].is_none() {
            state.keys[side] = Some(tree.map(Arc::new));
        }
        drop(state);
        self.changed.notify_all();
    }

    /// Phase 2 of `side`: the records it fetched (empty when nothing). The
    /// first submission counts.
    pub(super) fn submit_fetched(&self, side: usize, records: FolderCache) {
        let mut state = self.lock();
        if state.fetched[side].is_none() {
            state.fetched[side] = Some(Arc::new(records));
        }
        drop(state);
        self.changed.notify_all();
    }

    fn wait<T>(
        &self,
        stop: &AtomicBool,
        mut ready: impl FnMut(&mut State) -> Option<T>,
    ) -> io::Result<T> {
        let mut state = self.lock();
        loop {
            if let Some(value) = ready(&mut state) {
                return Ok(value);
            }
            if stop.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "synchronization tree walk canceled",
                ));
            }
            state = match self.changed.wait_timeout(state, WAIT_SLICE) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    /// The folders `side` sends its runs of, once both sides finished phase 1.
    pub(super) fn plan(&self, side: usize, stop: &AtomicBool) -> io::Result<Vec<String>> {
        let stored = [self.stored[0].clone(), self.stored[1].clone()];
        self.wait(stop, |state| {
            if state.plan.is_none() {
                let [Some(a), Some(b)] = [state.keys[0].clone(), state.keys[1].clone()] else {
                    return None;
                };
                state.plan = Some(plan([a, b], &stored));
            }
            state.plan.as_ref().map(|plan| plan[side].clone())
        })
    }

    /// Every record source once both sides finished phase 2: the stored
    /// keys of both sides, a local side's runs and what both sides fetched.
    pub(super) fn sources(&self, stop: &AtomicBool) -> io::Result<Vec<Arc<FolderCache>>> {
        let stored = [self.stored[0].clone(), self.stored[1].clone()];
        self.wait(stop, |state| {
            let [Some(a), Some(b)] = [state.fetched[0].clone(), state.fetched[1].clone()] else {
                return None;
            };
            let mut sources = vec![stored[0].clone(), stored[1].clone(), a, b];
            for tree in state.keys.iter().flatten().flatten() {
                if tree.records.len() > 0 {
                    sources.push(tree.records.clone());
                }
            }
            Some(sources)
        })
    }

    /// Declines whatever `side` did not submit yet (every exit of its walk).
    pub(super) fn finish(&self, side: usize) {
        self.submit_keys(side, None);
        self.submit_fetched(side, FolderCache::default());
    }
}

/// The folders each remote side sends: every key reachable from its root
/// that no complete stored or local subtree covers, each distinct key once.
fn plan(trees: [Option<Arc<SideTree>>; 2], stored: &[Arc<FolderCache>; 2]) -> [Vec<String>; 2] {
    let mut held: Vec<&FolderCache> = vec![&stored[0], &stored[1]];
    for tree in trees.iter().flatten() {
        held.push(&tree.records);
    }
    let covered = FolderCache::complete_keys(&held);
    // Remote sides: those whose tree came without records.
    let remote: [bool; 2] = [0, 1].map(|i| {
        trees[i]
            .as_ref()
            .is_some_and(|tree| tree.records.len() == 0)
    });
    let index: [HashMap<FolderKey, &str>; 2] = [0, 1].map(|i| {
        let mut index = HashMap::new();
        if let Some(tree) = trees[i].as_ref().filter(|_| remote[i]) {
            for (rel, key) in &tree.keys {
                index.entry(*key).or_insert(rel.as_str());
            }
        }
        index
    });
    let mut fetch: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    let mut assigned: HashSet<FolderKey> = HashSet::new();
    for side in 0..2 {
        let Some(tree) = trees[side].as_ref().filter(|_| remote[side]) else {
            continue;
        };
        let children = children_of(&tree.keys);
        let mut pending = VecDeque::from([String::new()]);
        while let Some(rel) = pending.pop_front() {
            let Some(key) = tree.keys.get(&rel) else {
                continue;
            };
            if covered.contains(key) {
                continue;
            }
            if assigned.insert(*key) {
                let other = 1 - side;
                match index[other].get(key) {
                    Some(at) if remote[other] && fetch[other].len() < fetch[side].len() => {
                        fetch[other].push((*at).to_string())
                    }
                    _ => fetch[side].push(rel.clone()),
                }
            }
            pending.extend(children.get(rel.as_str()).into_iter().flatten().cloned());
        }
    }
    fetch
}

/// The subfolders of every folder of a key tree.
fn children_of(keys: &HashMap<String, FolderKey>) -> HashMap<&str, Vec<String>> {
    let mut children: HashMap<&str, Vec<String>> = HashMap::new();
    for rel in keys.keys().filter(|rel| !rel.is_empty()) {
        let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
        children.entry(parent).or_default().push(rel.clone());
    }
    children
}

#[cfg(all(test, unix))]
#[path = "snapshot_pair_keys_tests.rs"]
mod tests;
