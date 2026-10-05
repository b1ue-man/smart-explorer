//! Read-only migration of the private four-map spelling format. Historical
//! folded keys prove only their recorded side paths, never new similar names.
use std::collections::{BTreeMap, BTreeSet};
use std::io;

use serde::{Deserialize, Serialize};

use super::keys::{KeyPolicy, PathAliases};
use super::run_types::StateKey;
use super::state_spellings::StateSpellings;
use super::types::PairSide;

#[derive(Default, Serialize, Deserialize)]
pub(super) struct LegacyAliases {
    pub files: BTreeMap<String, String>,
    pub dirs: BTreeMap<String, String>,
}

impl LegacyAliases {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.dirs.is_empty()
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(super) fn validate(
    value: &mut StateSpellings,
    state: &StateKey,
    keys: KeyPolicy,
) -> io::Result<()> {
    for (files, dirs) in [
        (&value.files_a, &value.dirs_a),
        (&value.files_b, &value.dirs_b),
    ] {
        if files.keys().any(|key| dirs.contains_key(key)) {
            return Err(invalid("one recorded slot is both a file and a folder"));
        }
    }
    let maps = [&value.files_a, &value.files_b, &value.dirs_a, &value.dirs_b];
    for map in maps {
        for rel in map.values() {
            super::sync_relative_path::SyncRelativePath::parse(rel)?;
        }
    }
    if value.legacy.is_empty()
        && maps
            .iter()
            .any(|map| map.iter().any(|(key, rel)| keys.key(rel).as_ref() != key))
    {
        migrate(value, state, keys)?;
    }
    // Provenance has its own bounded allowance; retaining an old valid map
    // must not consume that map's existing entry/text budget a second time.
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let mut entries = 0u64;
    let mut text = 0u64;
    for (key, rel) in value.legacy.files.iter().chain(&value.legacy.dirs) {
        entries = entries.saturating_add(1);
        text = text
            .saturating_add(key.len() as u64)
            .saturating_add(rel.len() as u64);
    }
    if entries > limits.state_entries.saturating_mul(2)
        || text > limits.state_text_bytes.saturating_mul(2)
    {
        return Err(invalid("historical spelling provenance exceeds its budget"));
    }
    let mut aliases = PathAliases::default();
    for (directory, anchors, a, b) in [
        (false, &value.legacy.files, &value.files_a, &value.files_b),
        (true, &value.legacy.dirs, &value.dirs_a, &value.dirs_b),
    ] {
        for (key, logical) in anchors {
            super::sync_relative_path::SyncRelativePath::parse(logical)?;
            if keys.key(logical).as_ref() != key || (!a.contains_key(key) && !b.contains_key(key)) {
                return Err(invalid("historical spelling has no matching recorded slot"));
            }
            let folded = KeyPolicy { fold_case: true };
            for (side, map) in [(PairSide::A, a), (PairSide::B, b)] {
                if let Some(physical) = map.get(key) {
                    if folded.key(physical) != folded.key(logical)
                        || !aliases.insert(side, logical, physical, directory, keys)
                    {
                        return Err(invalid(
                            "historical spelling relations contradict each other",
                        ));
                    }
                }
            }
        }
    }
    for (side, files, dirs) in [
        (PairSide::A, &value.files_a, &value.dirs_a),
        (PairSide::B, &value.files_b, &value.dirs_b),
    ] {
        for (key, rel) in files.iter().chain(dirs) {
            if aliases.key(rel, side, keys) != *key {
                return Err(invalid(
                    "stored spelling does not match its proven planning key",
                ));
            }
            if let Some((parent, _)) = rel.rsplit_once('/') {
                let logical = aliases.logical(rel, side, keys);
                let logical_parent = logical.rsplit_once('/').map(|(parent, _)| parent);
                if logical_parent.is_some_and(|logical| {
                    aliases.key(parent, side, keys) != keys.key(logical).as_ref()
                }) {
                    return Err(invalid("file and folder spelling relations disagree"));
                }
            }
        }
        if files.keys().any(|key| dirs.contains_key(key)) {
            return Err(invalid("one recorded slot is both a file and a folder"));
        }
    }
    value.aliases = aliases;
    Ok(())
}

fn migrate(value: &mut StateSpellings, state: &StateKey, keys: KeyPolicy) -> io::Result<()> {
    if keys.fold_case {
        return Err(invalid(
            "stored spelling does not match the folded pair policy",
        ));
    }
    let folded = KeyPolicy { fold_case: true };
    let (_, records, _) = super::checkpoint_journal::Journal::load(state, keys)?;
    // Directory history contains planning keys, not literal spellings.
    // OLDTREE and its confirmed ancestor OldTree are one relation; only
    // actual baseline ancestors can supply authoritative literal anchors.
    let mut dir_basis = BTreeSet::new();
    for rel in records.baseline.keys() {
        for (end, _) in rel.match_indices('/') {
            dir_basis.insert(rel[..end].to_string());
        }
    }
    let mut legacy = LegacyAliases::default();
    for (directory, a, b, anchors) in [
        (
            false,
            &mut value.files_a,
            &mut value.files_b,
            &mut legacy.files,
        ),
        (true, &mut value.dirs_a, &mut value.dirs_b, &mut legacy.dirs),
    ] {
        let groups: BTreeSet<_> = a.keys().chain(b.keys()).cloned().collect();
        let mut new_a = BTreeMap::new();
        let mut new_b = BTreeMap::new();
        for old in groups {
            let slots = [a.get(&old), b.get(&old)];
            let changed = slots
                .iter()
                .flatten()
                .any(|rel| keys.key(rel).as_ref() != old);
            let logical = if changed {
                if slots
                    .iter()
                    .flatten()
                    .any(|rel| folded.key(rel).as_ref() != old)
                {
                    return Err(invalid(
                        "stored key is neither current nor a historical folded key",
                    ));
                }
                let candidates: BTreeSet<_> = if directory {
                    dir_basis
                        .iter()
                        .filter(|rel| folded.key(rel).as_ref() == old)
                        .cloned()
                        .collect()
                } else {
                    records
                        .baseline
                        .keys()
                        .filter(|rel| folded.key(rel).as_ref() == old)
                        .cloned()
                        .collect()
                };
                if candidates.len() > 1 {
                    return Err(invalid(
                        "historical folded key has several authoritative spellings",
                    ));
                }
                // A stored A/B relation is evidence even for an unresolved
                // conflict with no baseline. No unrecorded sibling is joined.
                candidates
                    .into_iter()
                    .next()
                    .or_else(|| slots[0].or(slots[1]).cloned())
                    .ok_or_else(|| invalid("historical spelling has no side path"))?
            } else {
                slots[0]
                    .or(slots[1])
                    .cloned()
                    .ok_or_else(|| invalid("stored spelling has no side path"))?
            };
            let key = keys.key(&logical).into_owned();
            if (directory && changed)
                || slots
                    .iter()
                    .flatten()
                    .any(|rel| keys.key(rel).as_ref() != key)
            {
                anchors.insert(key.clone(), logical);
            }
            for (slot, map) in [(slots[0], &mut new_a), (slots[1], &mut new_b)] {
                if let Some(rel) = slot {
                    if map.insert(key.clone(), rel.clone()).is_some() {
                        return Err(invalid(
                            "historical spelling migration would merge recorded slots",
                        ));
                    }
                }
            }
        }
        *a = new_a;
        *b = new_b;
    }
    value.legacy = legacy;
    Ok(())
}
