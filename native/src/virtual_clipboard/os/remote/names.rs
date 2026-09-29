//! Windows' rules for the names Explorer creates from the descriptor: names
//! it cannot create at all, and names that differ only in case, which one
//! Windows folder cannot hold side by side.
use crate::vfs::remote_util::numbered_remote_name;
use std::collections::{HashMap, HashSet};

pub(super) const INVALID_NAME: &str = "Der Name enthält Zeichen, die Windows nicht zulässt";
pub(super) const RESERVED_NAME: &str =
    "Windows reserviert diesen Namen – in Smart Explorer einfügen überträgt ihn";
pub(super) const LONG_NAME: &str = "Name länger als 255 Zeichen – Windows kann ihn nicht anlegen";
/// NTFS, ReFS and FAT allow at most 255 UTF-16 units per name.
const PART_UNITS: usize = 255;

/// Why Explorer could not create `rel` as named, if so. Explorer turns the
/// descriptor name into a path as it is: a server's `..` would leave the
/// target folder, `name:stream` would write an NTFS stream, a backslash would
/// split the name, and Win32 strips trailing dots and spaces or opens a device.
pub(super) fn name_problem(rel: &str) -> Option<&'static str> {
    for part in rel.split('/') {
        let invalid = |c: char| c < ' ' || "<>:\"|?*\\".contains(c);
        if part.is_empty() || part == "." || part == ".." || part.contains(invalid) {
            return Some(INVALID_NAME);
        }
        if part.ends_with('.') || part.ends_with(' ') || is_device_name(part) {
            return Some(RESERVED_NAME);
        }
    }
    long_part(rel).then_some(LONG_NAME)
}

/// A name longer than a Windows file system allows.
pub(super) fn long_part(rel: &str) -> bool {
    rel.split('/')
        .any(|part| part.encode_utf16().count() > PART_UNITS)
}

/// Reserved device names (Microsoft's file naming rules), which Win32
/// resolves in any folder and with any extension.
fn is_device_name(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or(part)
        .trim_end()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let mut digit = stem.get(3..).unwrap_or_default().chars();
    matches!(stem.get(..3), Some("COM" | "LPT"))
        && matches!(digit.next(), Some('0'..='9' | '¹' | '²' | '³'))
        && digit.next().is_none()
}

/// Case-folds like the upcase table of NTFS: every character maps to its
/// simple uppercase form, one to one (no "ß" to "SS").
fn fold(text: &str) -> String {
    text.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(single), None) => single,
                _ => c,
            }
        })
        .collect()
}

pub(super) enum Placement {
    /// Hand the entry to Explorer under this relative path.
    At(String),
    /// A folder Windows already has under another case: its entries go there.
    Merged,
}

/// Places the listed entries (in listing order, folders before their
/// contents) under paths one Windows folder can hold: a file whose name
/// differs from an earlier one only in case is numbered like the transfer
/// walker numbers duplicates ("Name (2).ext"), and so is a folder that meets
/// a file; folders of equal names merge, as Windows merges them anyway.
pub(super) struct Namer {
    /// Every listed path, case-folded: numbered names avoid them.
    listed: HashSet<String>,
    /// Paths handed to Explorer, case-folded, with whether they are folders.
    placed: HashMap<String, bool>,
    /// Folders placed under another path: listed path → placed path.
    moved: HashMap<String, String>,
}

impl Namer {
    pub(super) fn new(listed: impl Iterator<Item = &str>) -> Self {
        Self {
            listed: listed.map(fold).collect(),
            placed: HashMap::new(),
            moved: HashMap::new(),
        }
    }

    pub(super) fn place(&mut self, rel: &str, is_dir: bool) -> Placement {
        let (parent, name) = match rel.rsplit_once('/') {
            Some((parent, name)) => (Some(parent), name),
            None => (None, rel),
        };
        let parent = parent.map(|parent| self.moved.get(parent).map_or(parent, String::as_str));
        let join = |name: &str| match parent {
            Some(parent) => format!("{parent}/{name}"),
            None => name.to_string(),
        };
        let mut at = join(name);
        match self.placed.get(&fold(&at)).copied() {
            None => {}
            Some(true) if is_dir => {
                if at != rel {
                    self.moved.insert(rel.to_string(), at);
                }
                return Placement::Merged;
            }
            Some(_) => {
                let free = (2..)
                    .map(|index| join(&numbered(name, index, is_dir)))
                    .find(|path| {
                        let key = fold(path);
                        !self.placed.contains_key(&key) && !self.listed.contains(&key)
                    });
                if let Some(free) = free {
                    at = free;
                }
            }
        }
        self.placed.insert(fold(&at), is_dir);
        if is_dir && at != rel {
            self.moved.insert(rel.to_string(), at.clone());
        }
        Placement::At(at)
    }
}

fn numbered(name: &str, index: usize, is_dir: bool) -> String {
    if is_dir {
        format!("{name} ({index})")
    } else {
        numbered_remote_name(name, index)
    }
}
