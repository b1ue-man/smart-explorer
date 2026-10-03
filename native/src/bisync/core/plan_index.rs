//! Both sides and the baseline joined by planning key (V3, Y37/Y48/Y102): each
//! side keeps its own spelling, two names one side cannot keep apart collide
//! and are protected instead of failing every run.
use std::collections::{BTreeMap, BTreeSet};

use super::keys::KeyPolicy;
use super::types::{Baseline, PairSide, Sig, Tree};

/// One planning key: what each side and the baseline hold under it.
#[derive(Clone, Debug, Default)]
pub(super) struct Keyed {
    pub(super) a: Option<(String, Sig)>,
    pub(super) b: Option<(String, Sig)>,
    pub(super) base: Option<(String, (Option<Sig>, Option<Sig>))>,
    /// Further baseline spellings of this key, dropped when it is planned.
    pub(super) stale: Vec<String>,
}

impl Keyed {
    fn slot(&mut self, side: PairSide) -> &mut Option<(String, Sig)> {
        match side {
            PairSide::A => &mut self.a,
            PairSide::B => &mut self.b,
        }
    }

    pub(super) fn sig(&self, side: PairSide) -> Option<Sig> {
        let slot = match side {
            PairSide::A => &self.a,
            PairSide::B => &self.b,
        };
        slot.as_ref().map(|(_, sig)| *sig)
    }

    pub(super) fn spelling(&self, side: PairSide) -> Option<&str> {
        let slot = match side {
            PairSide::A => &self.a,
            PairSide::B => &self.b,
        };
        slot.as_ref().map(|(rel, _)| rel.as_str())
    }

    pub(super) fn base_entry(&self) -> Option<(Option<Sig>, Option<Sig>)> {
        self.base.as_ref().map(|(_, entry)| *entry)
    }

    /// The rel actions and records use: side A's spelling, else side B's,
    /// else the baseline's.
    pub(super) fn rel(&self) -> &str {
        self.spelling(PairSide::A)
            .or_else(|| self.spelling(PairSide::B))
            .or_else(|| self.base.as_ref().map(|(rel, _)| rel.as_str()))
            .unwrap_or_default()
    }
}

pub(super) struct PairIndex {
    pub(super) entries: BTreeMap<String, Keyed>,
    /// Names that collide with another name of the same side under the key.
    pub(super) collisions: Vec<(PairSide, String)>,
}

pub(super) fn index(a: &Tree, b: &Tree, base: &Baseline, keys: KeyPolicy) -> PairIndex {
    let mut entries: BTreeMap<String, Keyed> = BTreeMap::new();
    let mut collisions = Vec::new();
    for (side, tree) in [(PairSide::A, a), (PairSide::B, b)] {
        let mut collided: BTreeSet<String> = BTreeSet::new();
        for (rel, sig) in tree {
            let key = keys.key(rel).into_owned();
            if collided.contains(&key) {
                collisions.push((side, rel.clone()));
                continue;
            }
            let slot = entries.entry(key.clone()).or_default().slot(side);
            match slot.take() {
                None => *slot = Some((rel.clone(), *sig)),
                Some((first, _)) => {
                    collisions.push((side, first));
                    collisions.push((side, rel.clone()));
                    collided.insert(key);
                }
            }
        }
    }
    for (rel, entry) in base {
        let keyed = entries.entry(keys.key(rel).into_owned()).or_default();
        let matches_side = keyed.spelling(PairSide::A) == Some(rel.as_str())
            || keyed.spelling(PairSide::B) == Some(rel.as_str());
        match keyed.base.take() {
            None => keyed.base = Some((rel.clone(), *entry)),
            Some(previous) if matches_side => {
                keyed.stale.push(previous.0);
                keyed.base = Some((rel.clone(), *entry));
            }
            Some(previous) => {
                keyed.base = Some(previous);
                keyed.stale.push(rel.clone());
            }
        }
    }
    PairIndex {
        entries,
        collisions,
    }
}

/// The path a new file gets on a side that does not have it yet: folders
/// the side already has keep that side's spelling (`photos/` stays
/// `photos/` for `Photos/new.jpg`), the rest keeps the source spelling.
pub(super) fn destination_spelling(
    rel: &str,
    dirs: &BTreeMap<String, String>,
    keys: KeyPolicy,
) -> String {
    let Some((parent, name)) = rel.rsplit_once('/') else {
        return rel.to_string();
    };
    let mut mapped = String::new();
    let mut source_prefix = String::new();
    let mut inside_existing = true;
    for part in parent.split('/') {
        if !source_prefix.is_empty() {
            source_prefix.push('/');
        }
        source_prefix.push_str(part);
        let existing = inside_existing
            .then(|| dirs.get(keys.key(&source_prefix).as_ref()))
            .flatten();
        match existing {
            Some(spelled) => mapped = spelled.clone(),
            None => {
                inside_existing = false;
                if !mapped.is_empty() {
                    mapped.push('/');
                }
                mapped.push_str(part);
            }
        }
    }
    format!("{mapped}/{name}")
}

/// Folders of one side by planning key (that side's spelling).
pub(super) fn dirs_by_key(
    dirs: &BTreeSet<String>,
    keys: KeyPolicy,
) -> BTreeMap<String, String> {
    dirs.iter()
        .map(|dir| (keys.key(dir).into_owned(), dir.clone()))
        .collect()
}
