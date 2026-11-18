//! Depth-first walk of the index from one directory record. Links are
//! leaves and never entered, every hard-linked file's occupied bytes count
//! once per walk, and a directory reached twice (a corrupt cycle) is a
//! defect instead of an endless loop.
use super::index::{Facts, MftIndex, DEFECT, DIRECTORY, PRESENT, REPARSE};
use super::{FileRef, MftDiagnostics, FIRST_USER_RECORD, ROOT_RECORD};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

const NAME_SURROGATE: u32 = 0x2000_0000;
/// AF_UNIX socket and WSL FIFO/character/block nodes (as `local_access`
/// `directory.rs` `SPECIAL_TAGS`).
const SPECIAL_TAGS: [u32; 4] = [0x8000_0023, 0x8000_0024, 0x8000_0025, 0x8000_0026];
const FILE_ATTRIBUTE_DEVICE: u32 = 0x40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MftKind {
    File,
    Directory,
    /// Symlink, junction, mount point or a reparse point whose tag is not
    /// in the record: never entered, no content, no bytes.
    Link,
    /// Socket, FIFO or device node: no bytes.
    Special,
}

/// One entry as the walk reaches it.
#[derive(Clone, Copy, Debug)]
pub struct MftEntry<'a> {
    pub name: &'a str,
    /// False when the NTFS name held unpaired surrogates and `name` is lossy.
    pub name_exact: bool,
    /// Directory names from the walk root (exclusive) down to the parent.
    pub parents: &'a [String],
    pub kind: MftKind,
    pub record: u64,
    /// Logical size of the unnamed data stream (0 for directories and links).
    pub size: u64,
    /// Occupied bytes: unnamed stream plus alternate data streams; 0 for a
    /// further link of a file already counted in this walk.
    pub allocated: u64,
    /// A further hard link of a file counted earlier in this walk.
    pub repeated_link: bool,
    /// `FILE_ATTRIBUTE_*` from `$STANDARD_INFORMATION`.
    pub attributes: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MftTotals {
    pub size: u64,
    pub allocated: u64,
    pub files: u64,
    pub directories: u64,
}

impl MftTotals {
    fn add(&mut self, other: &MftTotals) {
        self.size = self.size.saturating_add(other.size);
        self.allocated = self.allocated.saturating_add(other.allocated);
        self.files += other.files;
        self.directories += other.directories;
    }
}

/// Receives the walk in depth-first order.
pub trait MftVisitor {
    /// Whether to take the entry (and, for a directory, its subtree) into
    /// the walk: the place for the analysis exclusion rules. The root is
    /// never asked. `allocated` is shown before hard-link deduplication.
    fn keep(&mut self, _entry: &MftEntry<'_>) -> bool {
        true
    }
    fn enter_directory(&mut self, entry: &MftEntry<'_>);
    /// A file, link or special entry.
    fn leaf(&mut self, entry: &MftEntry<'_>);
    /// The directory entered last is complete.
    fn leave_directory(&mut self, totals: &MftTotals);
}

/// Which part of the volume root a walk from record 5 covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MftSection {
    /// Everything a directory listing shows.
    Content,
    /// Only the NTFS system files (`$MFT`, `$LogFile`, `$Extend`, ...).
    SystemFiles,
}

pub(super) fn kind_of(facts: &Facts) -> MftKind {
    let tag = facts.reparse_tag;
    if facts.flags & REPARSE != 0 && (tag == 0 || tag & NAME_SURROGATE != 0) {
        MftKind::Link
    } else if facts.flags & DIRECTORY != 0 {
        MftKind::Directory
    } else if facts.attributes & FILE_ATTRIBUTE_DEVICE != 0
        || (facts.flags & REPARSE != 0 && SPECIAL_TAGS.contains(&tag))
    {
        MftKind::Special
    } else {
        MftKind::File
    }
}

struct Bits(Vec<u64>);
impl Bits {
    fn new(count: usize) -> Self {
        Self(vec![0; count / 64 + 1])
    }
    /// Set bit `index`; false when it was set already.
    fn insert(&mut self, index: u32) -> bool {
        let (word, bit) = (index as usize / 64, 1u64 << (index % 64));
        match self.0.get_mut(word) {
            Some(value) if *value & bit == 0 => {
                *value |= bit;
                true
            }
            _ => false,
        }
    }
}

struct Frame {
    record: u32,
    next: usize,
    end: usize,
    totals: MftTotals,
}

fn decode(name: &[u16]) -> (String, bool) {
    match String::from_utf16(name) {
        Ok(text) => (text, true),
        Err(_) => (String::from_utf16_lossy(name), false),
    }
}

impl MftIndex {
    /// Walk the directory `root` (its sequence is checked unless 0). Errors:
    /// `NotFound` for a root that is no readable, in-use, non-link
    /// directory, `Interrupted` when `cancel` is set.
    pub fn walk(
        &self,
        root: FileRef,
        root_name: &str,
        section: MftSection,
        cancel: &AtomicBool,
        visitor: &mut dyn MftVisitor,
    ) -> io::Result<(MftTotals, MftDiagnostics)> {
        let missing = || io::Error::new(io::ErrorKind::NotFound, "Ordner nicht in der MFT");
        let start = u32::try_from(root.record).map_err(|_| missing())?;
        let facts = self.facts(start).ok_or_else(missing)?;
        if facts.flags & (PRESENT | DEFECT) != PRESENT
            || kind_of(facts) != MftKind::Directory
            || (root.sequence != 0 && root.sequence != facts.sequence)
        {
            return Err(missing());
        }
        let mut diagnostics = MftDiagnostics::default();
        let mut visited = Bits::new(self.facts.len());
        let mut counted = Bits::new(self.facts.len());
        let mut parents: Vec<String> = Vec::new();
        let mut stale = 0u64;
        visited.insert(start);
        visitor.enter_directory(&MftEntry {
            name: root_name,
            name_exact: true,
            parents: &[],
            kind: MftKind::Directory,
            record: root.record,
            size: 0,
            allocated: 0,
            repeated_link: false,
            attributes: facts.attributes,
        });
        let range = self.children(start);
        let mut stack = vec![Frame {
            record: start,
            next: range.start,
            end: range.end,
            totals: MftTotals {
                directories: 1,
                ..MftTotals::default()
            },
        }];
        loop {
            let Some(frame) = stack.last_mut() else {
                return Err(missing());
            };
            if frame.next >= frame.end {
                let totals = frame.totals;
                stack.pop();
                visitor.leave_directory(&totals);
                match stack.last_mut() {
                    Some(parent) => {
                        parent.totals.add(&totals);
                        parents.pop();
                    }
                    None => {
                        if stale > 0 {
                            diagnostics.note(format!(
                                "{stale} Einträge mit veraltetem Elternverweis übersprungen"
                            ));
                        }
                        return Ok((totals, diagnostics));
                    }
                }
                continue;
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "abgebrochen"));
            }
            let Some(link) = self.names.get(frame.next).copied() else {
                return Err(missing());
            };
            frame.next += 1;
            let parent_record = frame.record;
            if parent_record == ROOT_RECORD as u32 && root.record == ROOT_RECORD {
                let system = u64::from(link.child) < FIRST_USER_RECORD;
                if system != (section == MftSection::SystemFiles) {
                    continue;
                }
            }
            let parent_sequence = self.facts(parent_record).map_or(0, |facts| facts.sequence);
            if link.parent_sequence != 0 && link.parent_sequence != parent_sequence {
                stale += 1;
                continue;
            }
            let Some(child) = self.facts(link.child).copied() else {
                continue;
            };
            if child.flags & PRESENT == 0 {
                continue;
            }
            let kind = kind_of(&child);
            let (name, name_exact) = decode(self.name(&link));
            let (size, allocated) = match kind {
                MftKind::File => (child.size, child.allocated),
                _ => (0, 0),
            };
            let mut entry = MftEntry {
                name: &name,
                name_exact,
                parents: &parents,
                kind,
                record: u64::from(link.child),
                size,
                allocated,
                repeated_link: false,
                attributes: child.attributes,
            };
            if !visitor.keep(&entry) {
                continue;
            }
            if kind == MftKind::Directory {
                if !visited.insert(link.child) {
                    diagnostics.defect(format!(
                        "MFT-Ordner {} mehrfach erreicht (Zyklus); übersprungen",
                        link.child
                    ));
                    continue;
                }
                visitor.enter_directory(&entry);
                let range = self.children(link.child);
                stack.push(Frame {
                    record: link.child,
                    next: range.start,
                    end: range.end,
                    totals: MftTotals {
                        directories: 1,
                        ..MftTotals::default()
                    },
                });
                parents.push(name);
                continue;
            }
            if kind == MftKind::File && !counted.insert(link.child) {
                entry.allocated = 0;
                entry.repeated_link = true;
            }
            visitor.leaf(&entry);
            if let Some(frame) = stack.last_mut() {
                frame.totals.add(&MftTotals {
                    size: entry.size,
                    allocated: entry.allocated,
                    files: u64::from(kind == MftKind::File),
                    directories: 0,
                });
            }
        }
    }
}
