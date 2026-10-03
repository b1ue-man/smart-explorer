//! Persisted I/O spellings per side, indexed by the pair's planning keys.
//! The optional incremental cache must never treat a case-only rename as a
//! copy followed by deletion of the same destination object.
use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};

use super::keys::{KeyPolicy, Spellings};
use super::plan_index::destination_spelling;
use super::run_types::StateKey;
use super::snapshot_types::SideSnapshot;
use super::types::{Action, Baseline, PairSide};

#[derive(Default, Serialize, Deserialize)]
pub(super) struct StateSpellings {
    pub files_a: BTreeMap<String, String>,
    pub files_b: BTreeMap<String, String>,
    pub dirs_a: BTreeMap<String, String>,
    pub dirs_b: BTreeMap<String, String>,
}

impl StateSpellings {
    pub fn observe(&mut self, side: PairSide, snapshot: &SideSnapshot, keys: KeyPolicy) {
        let (files, dirs) = match side {
            PairSide::A => (&mut self.files_a, &mut self.dirs_a),
            PairSide::B => (&mut self.files_b, &mut self.dirs_b),
        };
        let present_files: std::collections::BTreeSet<_> = snapshot
            .tree
            .keys()
            .chain(snapshot.filtered.keys())
            .map(|rel| keys.key(rel).into_owned())
            .collect();
        files.retain(|key, rel| present_files.contains(key) || snapshot.omissions.protects(rel));
        for rel in snapshot.tree.keys().chain(snapshot.filtered.keys()) {
            files.insert(keys.key(rel).into_owned(), rel.clone());
        }
        let present: std::collections::BTreeSet<_> = snapshot
            .dirs
            .iter()
            .map(|rel| keys.key(rel).into_owned())
            .collect();
        dirs.retain(|key, rel| present.contains(key) || snapshot.omissions.protects(rel));
        for rel in &snapshot.dirs {
            dirs.insert(keys.key(rel).into_owned(), rel.clone());
        }
    }

    fn files(&self, side: PairSide) -> &BTreeMap<String, String> {
        match side {
            PairSide::A => &self.files_a,
            PairSide::B => &self.files_b,
        }
    }
    fn dirs(&self, side: PairSide) -> &BTreeMap<String, String> {
        match side {
            PairSide::A => &self.dirs_a,
            PairSide::B => &self.dirs_b,
        }
    }

    pub fn rel(&self, rel: &str, side: PairSide, keys: KeyPolicy) -> String {
        self.files(side)
            .get(keys.key(rel).as_ref())
            .cloned()
            .unwrap_or_else(|| destination_spelling(rel, self.dirs(side), keys))
    }

    pub fn for_actions(&self, actions: &[Action], source: PairSide, keys: KeyPolicy) -> Spellings {
        let mut spellings = Spellings::default();
        for rel in actions.iter().map(super::core::action_rel) {
            // Change feeds and source walks already supply the source's path.
            let target = self.rel(rel, source.other(), keys);
            spellings.insert(rel, source.other(), &target);
        }
        spellings
    }

    pub fn applied(
        &mut self,
        actions: &[Action],
        spellings: &Spellings,
        baseline: &Baseline,
        keys: KeyPolicy,
    ) {
        let by_key: BTreeMap<_, _> = baseline
            .iter()
            .map(|(rel, entry)| (keys.key(rel).into_owned(), *entry))
            .collect();
        for rel in actions.iter().map(super::core::action_rel) {
            let key = keys.key(rel).into_owned();
            // Only the authoritative checkpoint's existing signatures enter
            // the cache. Failed paths retain their previous spelling.
            let entry = by_key.get(&key).copied();
            if let Some((a, b)) = entry {
                for (side, sig) in [(PairSide::A, a), (PairSide::B, b)] {
                    if sig.is_some() {
                        let map = match side {
                            PairSide::A => &mut self.files_a,
                            PairSide::B => &mut self.files_b,
                        };
                        map.insert(key.clone(), spellings.side_rel(rel, side).to_string());
                    } else {
                        let map = match side {
                            PairSide::A => &mut self.files_a,
                            PairSide::B => &mut self.files_b,
                        };
                        map.remove(&key);
                    }
                }
            }
        }
        // Unresolved conflicts may have no baseline yet. Their observed
        // spellings stay available to resolve_recorded; SQL uses only baseline
        // signatures and is bootstrapped only after an entirely safe run.
    }

    /// The SQL rows retain each side's literal path; the baseline remains a
    /// single pair entry even when the two literal spellings differ.
    pub fn cache_baseline(&self, baseline: &Baseline, keys: KeyPolicy) -> Baseline {
        let mut rows = Baseline::new();
        for (rel, (a, b)) in baseline {
            if a.is_some() {
                rows.entry(self.rel(rel, PairSide::A, keys)).or_default().0 = *a;
            }
            if b.is_some() {
                rows.entry(self.rel(rel, PairSide::B, keys)).or_default().1 = *b;
            }
        }
        rows
    }
}

pub(super) fn load(key: &StateKey, keys: KeyPolicy) -> io::Result<StateSpellings> {
    let path = super::replica_state::baseline_file(key)?.with_extension("spellings.json");
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let value: StateSpellings =
        super::state_metadata::read_json(&path, limits.state_file_bytes().saturating_mul(4))?
            .unwrap_or_default();
    let mut entries = 0u64;
    let mut text = 0u64;
    for map in [&value.files_a, &value.files_b, &value.dirs_a, &value.dirs_b] {
        for (key, rel) in map {
            crate::agent_proto::ValidatedRelativePath::parse(rel)?;
            if keys.key(rel).as_ref() != key {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stored spelling does not match its planning key",
                ));
            }
            entries = entries.saturating_add(1);
            text = text
                .saturating_add(key.len() as u64)
                .saturating_add(rel.len() as u64);
        }
    }
    if entries > limits.state_entries.saturating_mul(2)
        || text > limits.state_text_bytes.saturating_mul(4)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "side spellings exceed their budget",
        ));
    }
    Ok(value)
}

pub(super) fn save(key: &StateKey, value: &StateSpellings) -> io::Result<()> {
    super::state_metadata::write_json(
        &super::replica_state::baseline_file(key)?.with_extension("spellings.json"),
        value,
    )
}
