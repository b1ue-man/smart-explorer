//! The same proven side mapping at the optional cache boundary. Physical
//! feed paths remain untouched; actions and cache proofs use pair keys.
use std::collections::{BTreeMap, BTreeSet};

use super::incremental_collect::ResolvedChange;
use super::keys::{KeyPolicy, PathAliases};
use super::state_spellings::StateSpellings;
use super::state_store::ItemRecord;
use super::types::{Baseline, PairSide};

pub(super) fn baseline_by_key(base: &Baseline, keys: KeyPolicy) -> Baseline {
    base.iter()
        .map(|(rel, entry)| (keys.key(rel).into_owned(), *entry))
        .collect()
}

pub(super) fn cache_by_key(
    a: &BTreeMap<String, ItemRecord>,
    b: &BTreeMap<String, ItemRecord>,
    keys: KeyPolicy,
    aliases: &PathAliases,
) -> Option<Baseline> {
    let mut base = Baseline::new();
    for (side, items) in [(PairSide::A, a), (PairSide::B, b)] {
        let mut seen = BTreeSet::new();
        for (rel, item) in items
            .iter()
            .filter(|(_, item)| !item.deleted && !item.is_dir)
        {
            let key = aliases.key(rel, side, keys);
            if !seen.insert(key.clone()) {
                return None;
            }
            let entry = base.entry(key).or_default();
            match side {
                PairSide::A => entry.0 = item.sig,
                PairSide::B => entry.1 = item.sig,
            }
        }
    }
    Some(base)
}

pub(super) fn logical_changes(
    changes: &[ResolvedChange],
    source: PairSide,
    keys: KeyPolicy,
    aliases: &PathAliases,
) -> Vec<ResolvedChange> {
    changes
        .iter()
        .cloned()
        .map(|mut change| {
            change.rel = aliases.logical(&change.rel, source, keys).into_owned();
            change.old_rel = change
                .old_rel
                .map(|old| aliases.logical(&old, source, keys).into_owned());
            change
        })
        .collect()
}

pub(super) fn target_available(
    rel: &str,
    target: PairSide,
    keys: KeyPolicy,
    names: &StateSpellings,
) -> bool {
    if names.aliases.is_empty() {
        return true;
    }
    let physical = names.rel(rel, target, keys);
    let wanted: Vec<_> = rel.split('/').collect();
    let actual: Vec<_> = physical.split('/').collect();
    wanted.len() == actual.len()
        && (1..=wanted.len()).all(|end| {
            names.aliases.key(&actual[..end].join("/"), target, keys)
                == keys.key(&wanted[..end].join("/")).as_ref()
        })
}
