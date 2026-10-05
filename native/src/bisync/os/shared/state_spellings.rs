//! Persisted I/O spellings per side, indexed by the pair's planning keys.
//! The optional incremental cache must never treat a case-only rename as a
//! copy followed by deletion of the same destination object.
use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};

use super::keys::{KeyPolicy, PathAliases, Spellings};
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
    #[serde(
        default,
        skip_serializing_if = "super::state_spelling_policy::LegacyAliases::is_empty"
    )]
    pub legacy: super::state_spelling_policy::LegacyAliases,
    #[serde(skip)]
    pub aliases: PathAliases,
}

impl StateSpellings {
    pub fn observe(&mut self, side: PairSide, snapshot: &SideSnapshot, keys: KeyPolicy) {
        let aliases = &self.aliases;
        let (files, dirs) = match side {
            PairSide::A => (&mut self.files_a, &mut self.dirs_a),
            PairSide::B => (&mut self.files_b, &mut self.dirs_b),
        };
        let present_files: std::collections::BTreeSet<_> = snapshot
            .tree
            .keys()
            .chain(snapshot.filtered.keys())
            .map(|rel| aliases.key(rel, side, keys))
            .collect();
        files.retain(|key, rel| present_files.contains(key) || snapshot.omissions.protects(rel));
        for rel in snapshot.tree.keys().chain(snapshot.filtered.keys()) {
            files.insert(aliases.key(rel, side, keys), rel.clone());
        }
        let present: std::collections::BTreeSet<_> = snapshot
            .dirs
            .iter()
            .map(|rel| aliases.key(rel, side, keys))
            .collect();
        dirs.retain(|key, rel| present.contains(key) || snapshot.omissions.protects(rel));
        for rel in &snapshot.dirs {
            dirs.insert(aliases.key(rel, side, keys), rel.clone());
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
            .unwrap_or_else(|| {
                let target = self.aliases.spelling(rel, side, keys);
                if target != rel {
                    target
                } else {
                    destination_spelling(rel, self.dirs(side), keys)
                }
            })
    }

    pub fn for_actions(&self, actions: &[Action], source: PairSide, keys: KeyPolicy) -> Spellings {
        let mut spellings = Spellings::default();
        for rel in actions.iter().map(super::core::action_rel) {
            let source_rel = self.rel(rel, source, keys);
            spellings.insert(rel, source, &source_rel);
            let target = self.rel(rel, source.other(), keys);
            spellings.insert(rel, source.other(), &target);
        }
        spellings
    }

    /// Recorded inputs can predate the policy change and name an exact old
    /// side slot. Normal new actions never use this compatibility lookup.
    pub fn recorded_rel(&self, rel: &str, keys: KeyPolicy) -> io::Result<String> {
        let key = keys.key(rel);
        let mut candidates = BTreeMap::new();
        if [&self.files_a, &self.files_b, &self.dirs_a, &self.dirs_b]
            .iter()
            .any(|map| map.contains_key(key.as_ref()))
        {
            candidates.insert(key.into_owned(), rel.to_string());
        }
        for side in [PairSide::A, PairSide::B] {
            let logical = self.aliases.logical(rel, side, keys);
            if logical.as_ref() != rel {
                candidates.insert(keys.key(&logical).into_owned(), logical.into_owned());
            }
        }
        if candidates.len() > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "recorded path has contradictory historical side relations",
            ));
        }
        Ok(candidates
            .into_values()
            .next()
            .unwrap_or_else(|| rel.to_string()))
    }

    pub fn applied(
        &mut self,
        actions: &[Action],
        spellings: &Spellings,
        previous: &Baseline,
        baseline: &Baseline,
        keys: KeyPolicy,
    ) {
        let previous_keys: std::collections::BTreeSet<_> = previous
            .keys()
            .map(|rel| keys.key(rel).into_owned())
            .collect();
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
            } else if previous_keys.contains(&key) {
                // A confirmed deletion retires its old file relation. Failed
                // actions still have their authoritative previous entry.
                self.files_a.remove(&key);
                self.files_b.remove(&key);
            }
        }
        // A failed child action can retain a file on a currently absent side.
        // Its recorded literal ancestors keep the existing directory relation
        // available for retry; no fresh case pairing is inferred here.
        for (files, dirs) in [
            (&self.files_a, &mut self.dirs_a),
            (&self.files_b, &mut self.dirs_b),
        ] {
            for (key, rel) in files {
                for ((key_end, _), (rel_end, _)) in
                    key.match_indices('/').zip(rel.match_indices('/'))
                {
                    if self.legacy.dirs.contains_key(&key[..key_end]) {
                        dirs.entry(key[..key_end].to_string())
                            .or_insert_with(|| rel[..rel_end].to_string());
                    }
                }
            }
        }
        // Unresolved conflicts may have no baseline yet. Their observed
        // spellings stay available to resolve_recorded; SQL uses only baseline
        // signatures and is bootstrapped only after an entirely safe run.
        self.legacy
            .files
            .retain(|key, _| self.files_a.contains_key(key) || self.files_b.contains_key(key));
        self.legacy
            .dirs
            .retain(|key, _| self.dirs_a.contains_key(key) || self.dirs_b.contains_key(key));
    }

    pub fn applied_directory_history(
        &mut self,
        previous: Option<&super::snapshot_types::DirSet>,
        current: &super::snapshot_types::DirSet,
    ) {
        let retired: Vec<_> = self
            .legacy
            .dirs
            .keys()
            .filter(|key| {
                previous.is_some_and(|dirs| dirs.contains(*key)) && !current.contains(*key)
            })
            .cloned()
            .collect();
        for key in retired {
            self.dirs_a.remove(&key);
            self.dirs_b.remove(&key);
            self.legacy.dirs.remove(&key);
        }
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
    let mut value: StateSpellings =
        super::state_metadata::read_json(&path, limits.state_file_bytes().saturating_mul(6))?
            .unwrap_or_default();
    let mut entries = 0u64;
    let mut text = 0u64;
    for map in [&value.files_a, &value.files_b, &value.dirs_a, &value.dirs_b] {
        for (key, rel) in map {
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
    super::state_spelling_policy::validate(&mut value, key, keys)?;
    Ok(value)
}

pub(super) fn save(key: &StateKey, value: &StateSpellings) -> io::Result<()> {
    super::state_metadata::write_json(
        &super::replica_state::baseline_file(key)?.with_extension("spellings.json"),
        value,
    )
}
