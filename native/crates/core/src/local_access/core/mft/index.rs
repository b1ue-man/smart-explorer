//! Collects parsed record segments into one compact per-record table plus
//! a name table sorted by parent, merging extension segments into their
//! base records and checking resident attribute lists.
use super::attributes::{AttributeList, Reparse};
use super::record::{FileName, ParsedRecord, RecordDefect, RecordState};
use super::{FileRef, MftDiagnostics, BAD_CLUSTERS_RECORD};
use std::collections::HashMap;
use std::io;
use std::ops::Range;

pub(super) const PRESENT: u8 = 1;
pub(super) const DIRECTORY: u8 = 2;
/// Reparse point; `reparse_tag == 0` means the tag is not known.
pub(super) const REPARSE: u8 = 4;
pub(super) const DEFECT: u8 = 8;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const NAMESPACE_DOS: u8 = 2;

/// What the walk needs of one file record (base plus extensions).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Facts {
    pub size: u64,
    pub allocated: u64,
    pub attributes: u32,
    pub reparse_tag: u32,
    pub sequence: u16,
    pub flags: u8,
}

/// One name of a record in one directory.
#[derive(Clone, Copy, Debug)]
pub(super) struct NameLink {
    pub start: u64,
    pub parent: u32,
    pub child: u32,
    pub parent_sequence: u16,
    pub len: u16,
}

/// Receives the parsed segments of one volume in any order.
pub struct MftIndexBuilder {
    facts: Vec<Facts>,
    names: Vec<NameLink>,
    arena: Vec<u16>,
    extensions: Vec<ParsedRecord>,
    lists: Vec<(u32, FileRef)>,
    diagnostics: MftDiagnostics,
}

/// The finished, walkable index of one volume's MFT.
pub struct MftIndex {
    pub(super) facts: Vec<Facts>,
    pub(super) names: Vec<NameLink>,
    arena: Vec<u16>,
    pub(super) diagnostics: MftDiagnostics,
}

impl MftIndexBuilder {
    /// An index for `records` record numbers; refuses MFTs whose numbers do
    /// not fit 32 bits and reports an allocation failure as an error.
    pub fn new(records: u64) -> io::Result<Self> {
        let count = u32::try_from(records)
            .map_err(|_| io::Error::new(io::ErrorKind::Unsupported, "MFT zu groß"))?;
        let mut facts = Vec::new();
        facts
            .try_reserve_exact(count as usize)
            .map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "MFT-Index"))?;
        facts.resize(count as usize, Facts::default());
        Ok(Self {
            facts,
            names: Vec::new(),
            arena: Vec::new(),
            extensions: Vec::new(),
            lists: Vec::new(),
            diagnostics: MftDiagnostics::default(),
        })
    }

    /// Record the parse result of segment `number`.
    pub fn add(&mut self, number: u64, parsed: Result<RecordState, RecordDefect>) {
        let Some(slot) = usize::try_from(number)
            .ok()
            .filter(|n| *n < self.facts.len())
        else {
            return;
        };
        match parsed {
            Err(defect) => {
                self.facts[slot].flags |= DEFECT;
                self.diagnostics
                    .defect(format!("MFT-Datensatz {number}: {defect}"));
            }
            Ok(RecordState::Unused) => {}
            Ok(RecordState::InUse(record)) if record.base.is_some() => {
                self.extensions.push(record);
            }
            Ok(RecordState::InUse(record)) => self.apply_base(slot, record),
        }
    }

    fn apply_base(&mut self, slot: usize, record: ParsedRecord) {
        let facts = &mut self.facts[slot];
        facts.flags |= PRESENT;
        if record.is_directory {
            facts.flags |= DIRECTORY;
        }
        facts.sequence = record.sequence;
        facts.attributes = record.attributes.unwrap_or(0);
        if let Some(AttributeList::Segments(segments)) = &record.attribute_list {
            self.lists
                .extend(segments.iter().map(|segment| (slot as u32, *segment)));
        }
        self.merge(slot, &record);
    }

    /// Sizes, reparse data and names of one segment into its file.
    fn merge(&mut self, slot: usize, record: &ParsedRecord) {
        let facts = &mut self.facts[slot];
        if let Some(data) = record.data {
            facts.size = data.size;
            facts.allocated = facts.allocated.saturating_add(data.allocated);
        }
        // $BadClus:$Bad spans the whole volume and holds no data.
        if slot as u64 != BAD_CLUSTERS_RECORD {
            facts.allocated = facts.allocated.saturating_add(record.streams_allocated);
        }
        match record.reparse {
            Some(Reparse::Tag(tag)) => {
                facts.flags |= REPARSE;
                facts.reparse_tag = tag;
            }
            Some(Reparse::Unknown) => {
                facts.flags |= REPARSE;
                facts.reparse_tag = 0;
            }
            None => {}
        }
        self.add_names(slot as u32, &record.names);
    }

    fn add_names(&mut self, child: u32, names: &[FileName]) {
        for (index, name) in names.iter().enumerate() {
            // The 8.3 alias duplicates a long name; the root names itself ".".
            if name.namespace == NAMESPACE_DOS || u64::from(child) == name.parent.record {
                continue;
            }
            let duplicate = names[..index]
                .iter()
                .any(|other| other.parent == name.parent && other.name == name.name);
            let Ok(parent) = u32::try_from(name.parent.record) else {
                self.diagnostics.note(format!(
                    "MFT-Datensatz {child}: Elternverweis {} außerhalb der MFT",
                    name.parent.record
                ));
                continue;
            };
            if duplicate {
                continue;
            }
            self.names.push(NameLink {
                start: self.arena.len() as u64,
                parent,
                child,
                parent_sequence: name.parent.sequence,
                len: name.name.len() as u16,
            });
            self.arena.extend_from_slice(&name.name);
        }
    }

    /// A note that hides no bytes of the read records (`partial` unchanged).
    pub(super) fn note(&mut self, text: String) {
        self.diagnostics.note(text);
    }

    /// Merge extensions, check attribute lists and sort the names by parent.
    pub fn finish(mut self) -> MftIndex {
        let mut seen: HashMap<u64, (u16, u64)> = HashMap::new();
        for record in std::mem::take(&mut self.extensions) {
            let Some(base) = record.base else { continue };
            seen.insert(record.number, (record.sequence, base.record));
            let valid = usize::try_from(base.record).ok().filter(|slot| {
                self.facts.get(*slot).is_some_and(|facts| {
                    facts.flags & PRESENT != 0 && facts.sequence == base.sequence
                })
            });
            match valid {
                Some(slot) => self.merge(slot, &record),
                None => self.diagnostics.note(format!(
                    "MFT-Erweiterungsdatensatz {} gehört zu keinem gültigen Basisdatensatz {}",
                    record.number, base.record
                )),
            }
        }
        for (base, segment) in std::mem::take(&mut self.lists) {
            if segment.record == u64::from(base) {
                continue;
            }
            let found = seen.get(&segment.record).is_some_and(|(sequence, owner)| {
                *owner == u64::from(base)
                    && (segment.sequence == 0 || *sequence == segment.sequence)
            });
            if !found {
                self.diagnostics.defect(format!(
                    "MFT-Datensatz {base}: Erweiterungsdatensatz {} fehlt oder ist unlesbar",
                    segment.record
                ));
            }
        }
        for facts in &mut self.facts {
            if facts.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && facts.flags & REPARSE == 0 {
                facts.flags |= REPARSE;
                facts.reparse_tag = 0;
            }
        }
        let orphans = self
            .names
            .iter()
            .filter(|link| {
                !self.facts.get(link.parent as usize).is_some_and(|parent| {
                    parent.flags & (PRESENT | DIRECTORY) == PRESENT | DIRECTORY
                        && (link.parent_sequence == 0 || parent.sequence == link.parent_sequence)
                })
            })
            .count();
        if orphans > 0 {
            // chkdsk territory: these names are in no directory a walk reaches.
            self.diagnostics.note(format!(
                "{orphans} MFT-Namen ohne gültigen Elternordner (verwaist, nicht gezählt)"
            ));
        }
        let arena = std::mem::take(&mut self.arena);
        let text = |link: &NameLink| name_of(&arena, link);
        self.names
            .sort_unstable_by(|a, b| a.parent.cmp(&b.parent).then_with(|| text(a).cmp(text(b))));
        MftIndex {
            facts: self.facts,
            names: self.names,
            arena,
            diagnostics: self.diagnostics,
        }
    }
}

fn name_of<'a>(arena: &'a [u16], link: &NameLink) -> &'a [u16] {
    let start = usize::try_from(link.start).unwrap_or(usize::MAX);
    arena
        .get(start..start.saturating_add(usize::from(link.len)))
        .unwrap_or(&[])
}

impl MftIndex {
    /// Record numbers the index covers.
    pub fn record_count(&self) -> u64 {
        self.facts.len() as u64
    }

    /// Defects and notes found while reading the MFT.
    pub fn diagnostics(&self) -> &MftDiagnostics {
        &self.diagnostics
    }

    /// The current reference of an in-use base record.
    pub fn file_ref(&self, record: u64) -> Option<FileRef> {
        let facts = self.facts.get(usize::try_from(record).ok()?)?;
        (facts.flags & PRESENT != 0).then_some(FileRef {
            record,
            sequence: facts.sequence,
        })
    }

    pub(super) fn facts(&self, record: u32) -> Option<&Facts> {
        self.facts.get(record as usize)
    }

    pub(super) fn name(&self, link: &NameLink) -> &[u16] {
        name_of(&self.arena, link)
    }

    /// Indices into `names` of the entries of directory `parent`.
    pub(super) fn children(&self, parent: u32) -> Range<usize> {
        let start = self.names.partition_point(|link| link.parent < parent);
        let end = self.names.partition_point(|link| link.parent <= parent);
        start..end
    }
}
