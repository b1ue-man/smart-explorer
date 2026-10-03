//! Planning keys of one pair (V3, Y37/Y48/Y102): two spellings of one entry
//! meet under one key (NFC always, letter case folded when either side does
//! not distinguish case), while each side keeps its own spelling for I/O.
use std::borrow::Cow;
use std::collections::BTreeMap;

use super::types::PairSide;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyPolicy {
    /// Either side ignores letter case (NTFS, exFAT, FAT, SMB, Android
    /// shared storage, Drive): `Photo.JPG` and `photo.jpg` are one entry.
    pub fold_case: bool,
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
