//! Planning keys of one pair (V3, Y37/Y48/Y102): two spellings of one entry
//! meet under one key (NFC always, letter case folded when either side does
//! not distinguish case), while each side keeps its own spelling for I/O.
use std::borrow::Cow;
use std::collections::BTreeMap;

use super::types::PairSide;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyPolicy {
    /// Either side ignores letter case (NTFS, exFAT, FAT, SMB, Android
    /// shared storage): `Photo.JPG` and `photo.jpg` are one entry.
    pub fold_case: bool,
}

/// Exact, previously recorded side paths. Only these proven relations may
/// outlive a change from folded to case-sensitive pair keys.
#[derive(Clone, Debug, Default)]
pub(crate) struct PathAliases {
    a: AliasSide,
    b: AliasSide,
}

#[derive(Clone, Debug, Default)]
struct AliasSide {
    files: BTreeMap<String, String>,
    dirs: BTreeMap<String, String>,
    file_targets: BTreeMap<String, String>,
    dir_targets: BTreeMap<String, String>,
}

impl PathAliases {
    pub(crate) fn is_empty(&self) -> bool {
        self.a.files.is_empty()
            && self.a.dirs.is_empty()
            && self.b.files.is_empty()
            && self.b.dirs.is_empty()
    }

    fn side(&self, side: PairSide) -> &AliasSide {
        match side {
            PairSide::A => &self.a,
            PairSide::B => &self.b,
        }
    }

    pub(crate) fn insert(
        &mut self,
        side: PairSide,
        logical: &str,
        physical: &str,
        directory: bool,
        keys: KeyPolicy,
    ) -> bool {
        let side = match side {
            PairSide::A => &mut self.a,
            PairSide::B => &mut self.b,
        };
        let (sources, targets) = if directory {
            (&mut side.dirs, &mut side.dir_targets)
        } else {
            (&mut side.files, &mut side.file_targets)
        };
        let source = keys.key(physical).into_owned();
        let target = keys.key(logical).into_owned();
        if sources
            .get(&source)
            .is_some_and(|old| keys.key(old).as_ref() != target)
            || targets
                .get(&target)
                .is_some_and(|old| keys.key(old).as_ref() != source)
        {
            return false;
        }
        sources.insert(source, logical.to_string());
        targets.insert(target, physical.to_string());
        true
    }

    pub(crate) fn logical<'a>(
        &self,
        rel: &'a str,
        side: PairSide,
        keys: KeyPolicy,
    ) -> Cow<'a, str> {
        let aliases = self.side(side);
        let key = keys.key(rel);
        if let Some(logical) = aliases
            .files
            .get(key.as_ref())
            .or_else(|| aliases.dirs.get(key.as_ref()))
        {
            return Cow::Owned(logical.clone());
        }
        for ((raw_end, _), (key_end, _)) in rel
            .match_indices('/')
            .rev()
            .zip(key.match_indices('/').rev())
        {
            if let Some(parent) = aliases.dirs.get(&key[..key_end]) {
                return Cow::Owned(format!("{parent}{}", &rel[raw_end..]));
            }
        }
        Cow::Borrowed(rel)
    }

    pub(crate) fn key(&self, rel: &str, side: PairSide, keys: KeyPolicy) -> String {
        keys.key(&self.logical(rel, side, keys)).into_owned()
    }

    pub(crate) fn spelling(&self, rel: &str, side: PairSide, keys: KeyPolicy) -> String {
        let aliases = self.side(side);
        let key = keys.key(rel);
        if let Some(physical) = aliases
            .file_targets
            .get(key.as_ref())
            .or_else(|| aliases.dir_targets.get(key.as_ref()))
        {
            return physical.clone();
        }
        for ((raw_end, _), (key_end, _)) in rel
            .match_indices('/')
            .rev()
            .zip(key.match_indices('/').rev())
        {
            if let Some(parent) = aliases.dir_targets.get(&key[..key_end]) {
                return format!("{parent}{}", &rel[raw_end..]);
            }
        }
        rel.to_string()
    }

    pub(crate) fn protect_counterparts(
        &self,
        omissions: &mut super::omissions::SyncOmissions,
        keys: KeyPolicy,
    ) {
        if self.is_empty() {
            return;
        }
        let paths: Vec<_> = omissions
            .paths()
            .map(|(rel, kind, reported)| (rel.to_string(), kind, reported))
            .collect();
        for (rel, kind, reported) in paths {
            for side in [PairSide::A, PairSide::B] {
                let logical = self.logical(&rel, side, keys);
                omissions.record_kind(&logical, kind, reported);
                for target in [PairSide::A, PairSide::B] {
                    omissions.record_kind(
                        &self.spelling(&logical, target, keys),
                        kind,
                        reported,
                    );
                }
            }
        }
    }
}

impl KeyPolicy {
    pub fn for_pair(a_case_sensitive: bool, b_case_sensitive: bool) -> Self {
        Self {
            fold_case: !a_case_sensitive || !b_case_sensitive,
        }
    }

    /// The key all spellings of one entry share: Unicode NFC, then (when
    /// folding) each character's simple uppercase mapping, as the upcase
    /// tables of NTFS/exFAT/FAT compare names. Borrowed when unchanged.
    pub fn key<'a>(&self, rel: &'a str) -> Cow<'a, str> {
        let normalized = icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(rel);
        if !self.fold_case || !normalized.chars().any(|c| fold_char(c) != c) {
            return normalized;
        }
        Cow::Owned(normalized.chars().map(fold_char).collect())
    }
}

/// Simple (one-to-one) uppercase mapping; characters whose uppercase form has
/// several characters (`ß`) stay as they are, as on NTFS.
fn fold_char(c: char) -> char {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// Each side's own spelling of planned paths whose I/O path differs from the
/// action's rel (the action's rel is side A's spelling, or side B's when A
/// has none). Apply reads and writes `side_rel(rel, side)` (V3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spellings {
    a: BTreeMap<String, String>,
    b: BTreeMap<String, String>,
}

impl Spellings {
    /// Records `spelling` as `side`'s path of `rel`; equal spellings are not
    /// stored.
    pub fn insert(&mut self, rel: &str, side: PairSide, spelling: &str) {
        if rel == spelling {
            return;
        }
        let map = match side {
            PairSide::A => &mut self.a,
            PairSide::B => &mut self.b,
        };
        map.insert(rel.to_string(), spelling.to_string());
    }

    /// The path of `rel` on `side` (relative to that side's root).
    pub fn side_rel<'a>(&'a self, rel: &'a str, side: PairSide) -> &'a str {
        let map = match side {
            PairSide::A => &self.a,
            PairSide::B => &self.b,
        };
        map.get(rel).map_or(rel, String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.a.is_empty() && self.b.is_empty()
    }
}
