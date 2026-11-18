//! NTFS master file table (MFT) parsing for the size analysis: pure
//! functions on bytes, the same on every platform so they are tested
//! everywhere. Reading the volume is Windows-only
//! (`local_access::platform::mft`, behind `LocalElevatedScan`); nothing
//! here opens a file.
//!
//! Counting rules: logical size is the unnamed `$DATA` stream; occupied
//! bytes are the unnamed stream's allocation (compressed and sparse
//! streams: the clusters really held) plus every named stream (alternate
//! data streams, including `WofCompressedData` of WOF-compressed files).
//! A file with several names is counted with its occupied bytes once per
//! walk. Reparse points with the name-surrogate bit (symlinks, junctions,
//! mount points) are leaves without bytes and never entered. A defective
//! in-use record marks the result partial.
mod attributes;
mod bytes;
mod index;
mod layout;
mod record;
mod stream;
mod tree;
mod walk;

#[cfg(test)]
mod mft_task_fixtures;
#[cfg(test)]
mod mft_task_tests;

pub use attributes::{AttributeList, Reparse, StreamSize};
pub use index::{MftIndex, MftIndexBuilder};
pub use layout::{
    parse_retrieval_pointers, parse_volume_data, plan_reads, MftExtent, NtfsVolumeData, ReadSpan,
    VOLUME_DATA_LEN,
};
pub use record::{parse_record, FileName, ParsedRecord, RecordDefect, RecordState};
pub use stream::MftRecordReader;
pub use tree::{MftNode, MftScan, SYSTEM_FILES_NAME};
pub use walk::{MftEntry, MftKind, MftSection, MftTotals, MftVisitor};

/// Record of the volume's root directory.
pub const ROOT_RECORD: u64 = 5;
/// Records below this number are NTFS system files (`$MFT` ... `$Extend`
/// and reserved slots).
pub const FIRST_USER_RECORD: u64 = 16;
/// `$BadClus`, whose `$Bad` stream spans the volume without holding data.
const BAD_CLUSTERS_RECORD: u64 = 8;
/// Notes kept verbatim; further ones are only counted.
const MAX_NOTES: usize = 64;
/// Diagnostic note when the volume cache could not be flushed before reading.
pub const UNFLUSHED_NOTE: &str =
    "kürzlich geänderte Dateien sind möglicherweise noch nicht enthalten";

/// A 64-bit NTFS file reference: 48-bit record number, 16-bit sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FileRef {
    pub record: u64,
    pub sequence: u16,
}

impl FileRef {
    pub fn from_raw(raw: u64) -> Self {
        Self {
            record: raw & 0x0000_FFFF_FFFF_FFFF,
            sequence: (raw >> 48) as u16,
        }
    }

    pub fn raw(self) -> u64 {
        (self.record & 0x0000_FFFF_FFFF_FFFF) | (u64::from(self.sequence) << 48)
    }
}

/// What the scan could not read. `partial` is set by every defect that can
/// hide bytes; plain notes (orphans, stale extensions) leave it unset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MftDiagnostics {
    pub partial: bool,
    pub notes: Vec<String>,
    /// Notes beyond the first `MAX_NOTES`, counted only.
    pub omitted_notes: u64,
}

impl MftDiagnostics {
    pub(crate) fn defect(&mut self, text: String) {
        self.partial = true;
        self.note(text);
    }

    pub(crate) fn note(&mut self, text: String) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(text);
        } else {
            self.omitted_notes += 1;
        }
    }

    pub(crate) fn absorb(&mut self, other: MftDiagnostics) {
        self.partial |= other.partial;
        self.omitted_notes += other.omitted_notes;
        for note in other.notes {
            self.note(note);
        }
    }
}
