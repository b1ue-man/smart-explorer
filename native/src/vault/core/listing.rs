//! Rules for the entries of a content directory (format 8) as the vault
//! layer lists them: which stored names belong to the vault, how foreign
//! conflict copies get a display name without being renamed (C1.8), which
//! node shapes are valid, and the texts of entries that cannot be listed
//! (C1.9). Pure functions; the backend does the I/O.
use std::collections::{HashMap, HashSet};

use crate::cryptomator::{CIPHER_SUFFIX, DIR_ID_BACKUP_FILE, SHORTENED_SUFFIX};

/// At most this many encrypted-name candidates are tried per foreign name,
/// so a hostile name cannot make a listing slow.
pub(crate) const MAX_CONFLICT_CANDIDATES: usize = 64;

/// Omission texts: never a cleartext name (K24), only what went wrong.
pub(crate) const UNDECRYPTABLE: &str = "Name nicht entschlüsselbar";
pub(crate) const LONG_NAME_BROKEN: &str = "Langer Name fehlt oder ist beschädigt";
pub(crate) const LONG_NAME_MISMATCH: &str = "Kurzname passt nicht zum langen Namen";
pub(crate) const UNTYPED_NODE: &str = "Eintrag ohne Typdatei (unvollständig oder beschädigt)";
pub(crate) const AMBIGUOUS_NODE: &str = "Mehrdeutiger Eintrag (mehrere Typdateien)";
pub(crate) const BAD_SIZE: &str = "Chiffratgröße ungültig";
pub(crate) const STORAGE_LINK: &str = "Verknüpfung im Tresorspeicher";
pub(crate) const STORAGE_SPECIAL: &str = "Sonderdatei im Tresorspeicher";

/// What a name in a content directory is to the vault layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryClass {
    /// `<name>.c9r`: a node with an encrypted name (canonical or a
    /// foreign conflict copy).
    Short,
    /// `<name>.c9s`: a node with a shortened name (`name.c9s` inside).
    Shortened,
    /// `dirid.c9r`, stages, `.c9u` markers and foreign files: not listed.
    Ignored,
}

pub(crate) fn classify_entry(name: &str) -> EntryClass {
    if name == DIR_ID_BACKUP_FILE {
        EntryClass::Ignored
    } else if name.len() > CIPHER_SUFFIX.len() && name.ends_with(CIPHER_SUFFIX) {
        EntryClass::Short
    } else if name.len() > SHORTENED_SUFFIX.len() && name.ends_with(SHORTENED_SUFFIX) {
        EntryClass::Shortened
    } else {
        EntryClass::Ignored
    }
}

fn is_base64(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

/// The leftmost, longest match of CryptoFS's `BASE64_PATTERN`
/// (`([a-zA-Z0-9-_]{4})*[a-zA-Z0-9-_]{20}[a-zA-Z0-9-_=]{4}`) inside
/// `bytes[start..end]`, as `(from, to)`.
fn find_base64(bytes: &[u8], start: usize, end: usize) -> Option<(usize, usize)> {
    for from in start..end {
        let mut run = from;
        while run < end && is_base64(bytes[run]) {
            run += 1;
        }
        let mut length = ((end - from).min(run - from + 4) / 4) * 4;
        while length >= 24 {
            let tail = from + length - 4;
            if tail <= run
                && bytes[tail..from + length]
                    .iter()
                    .all(|byte| is_base64(*byte) || *byte == b'=')
            {
                return Some((from, from + length));
            }
            length -= 4;
        }
    }
    None
}

/// The encrypted-name candidates inside the base name (without `.c9r`) of
/// a foreign conflict copy, in the order CryptoFS's `C9rDecryptor` tries
/// them: the longest match at the leftmost position, then shorter matches
/// at that position, then later positions; each candidate once, at most
/// `MAX_CONFLICT_CANDIDATES`.
pub(crate) fn conflict_candidates(basename: &str) -> Vec<&str> {
    let bytes = basename.as_bytes();
    let mut seen = HashSet::new();
    let mut visited = HashSet::new();
    let mut found = Vec::new();
    let mut regions = vec![(0, bytes.len())];
    while let Some((start, end)) = regions.pop() {
        if found.len() >= MAX_CONFLICT_CANDIDATES || visited.len() >= 4 * MAX_CONFLICT_CANDIDATES {
            break;
        }
        if !visited.insert((start, end)) {
            continue;
        }
        let Some((from, to)) = find_base64(bytes, start, end) else {
            continue;
        };
        if seen.insert((from, to)) {
            found.push(&basename[from..to]);
        }
        // Depth first like the recursion: shorter match first, then later.
        regions.push((from + 1, end));
        regions.push((from, to - 1));
    }
    found
}

/// The display name of a conflict copy whose name decrypts to `cleartext`:
/// `cleartext` itself while no listed entry has it, else CryptoFS's
/// `<base> (<n>)<extension>` with the smallest free `n`. Nothing is renamed.
pub(crate) fn alias_name(cleartext: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(cleartext) {
        return cleartext.to_string();
    }
    let (base, extension) = match cleartext.rfind('.').filter(|index| *index > 0) {
        Some(index) => cleartext.split_at(index),
        None => (cleartext, ""),
    };
    (1u64..)
        .map(|number| format!("{base} ({number}){extension}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| format!("{base} ({}){extension}", u64::MAX))
}

/// Where a listed display name really is: the stored entry in its content
/// directory (a conflict copy or a name not stored canonically).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AliasTarget {
    pub(crate) stored: String,
    pub(crate) shortened: bool,
    /// The full encrypted name (`<base64>.c9r`) the entry decrypts with.
    pub(crate) encrypted: String,
}

/// Display name → stored entry of one folder.
pub(crate) type AliasMap = HashMap<String, AliasTarget>;

/// The type of a node folder, from the type files it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeShape {
    Dir,
    Symlink,
    /// Only for shortened nodes: `contents.c9r`.
    File,
    Ambiguous,
    Untyped,
}

/// The shape of a node folder holding the given type files. A short node
/// (`<name>.c9r/`) is a folder or a link; only a shortened node can also be
/// a file (`contents.c9r`).
pub(crate) fn node_shape(
    shortened: bool,
    dir_file: bool,
    symlink_file: bool,
    contents_file: bool,
) -> NodeShape {
    let contents_file = shortened && contents_file;
    match (dir_file, symlink_file, contents_file) {
        (true, false, false) => NodeShape::Dir,
        (false, true, false) => NodeShape::Symlink,
        (false, false, true) => NodeShape::File,
        (false, false, false) => NodeShape::Untyped,
        _ => NodeShape::Ambiguous,
    }
}
