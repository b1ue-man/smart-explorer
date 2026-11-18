//! Builders for synthetic MFT record bytes, laid out as NTFS 3.1 writes
//! them (1 KiB records, update sequence at 0x30).
use super::{FileRef, MftIndex, MftRecordReader, NtfsVolumeData};

pub(super) const RECORD: usize = 1024;
const USA_OFFSET: usize = 0x30;
const USN: u16 = 0x0A0B;
pub(super) const WIN32: u8 = 1;
pub(super) const DOS: u8 = 2;
pub(super) const WIN32_AND_DOS: u8 = 3;
pub(super) const POSIX: u8 = 0;

fn align8(value: usize) -> usize {
    (value + 7) & !7
}

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn utf16_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

pub(super) fn reference(record: u64, sequence: u16) -> FileRef {
    FileRef { record, sequence }
}

fn resident(type_code: u32, name: &str, value: &[u8]) -> Vec<u8> {
    let name = utf16_bytes(name);
    let value_offset = align8(0x18 + name.len());
    let mut bytes = vec![0u8; align8(value_offset + value.len())];
    put32(&mut bytes, 0, type_code);
    let length = bytes.len() as u32;
    put32(&mut bytes, 4, length);
    bytes[9] = (name.len() / 2) as u8;
    put16(&mut bytes, 0x0A, 0x18);
    put32(&mut bytes, 0x10, value.len() as u32);
    put16(&mut bytes, 0x14, value_offset as u16);
    bytes[0x18..0x18 + name.len()].copy_from_slice(&name);
    bytes[value_offset..value_offset + value.len()].copy_from_slice(value);
    bytes
}

/// Non-resident `$DATA` with an empty mapping-pairs array.
fn non_resident(
    name: &str,
    lowest_vcn: u64,
    allocated: u64,
    size: u64,
    compressed: Option<u64>,
) -> Vec<u8> {
    let name = utf16_bytes(name);
    let header = if compressed.is_some() { 0x48 } else { 0x40 };
    let runs = align8(header + name.len());
    let mut bytes = vec![0u8; align8(runs + 1)];
    put32(&mut bytes, 0, 0x80);
    let length = bytes.len() as u32;
    put32(&mut bytes, 4, length);
    bytes[8] = 1;
    bytes[9] = (name.len() / 2) as u8;
    put16(&mut bytes, 0x0A, header as u16);
    // Sparse flag: the header then carries the compressed (held) size.
    put16(
        &mut bytes,
        0x0C,
        if compressed.is_some() { 0x8000 } else { 0 },
    );
    put64(&mut bytes, 0x10, lowest_vcn);
    put16(&mut bytes, 0x20, runs as u16);
    put64(&mut bytes, 0x28, allocated);
    put64(&mut bytes, 0x30, size);
    put64(&mut bytes, 0x38, size);
    if let Some(held) = compressed {
        put64(&mut bytes, 0x40, held);
    }
    bytes[header..header + name.len()].copy_from_slice(&name);
    bytes
}

/// One record segment under construction.
pub(super) struct Rec {
    pub number: u64,
    sequence: u16,
    flags: u16,
    base: u64,
    attributes: Vec<Vec<u8>>,
}

impl Rec {
    pub fn file(number: u64) -> Self {
        Self {
            number,
            sequence: 1,
            flags: 1,
            base: 0,
            attributes: Vec::new(),
        }
    }

    pub fn dir(number: u64) -> Self {
        Self {
            flags: 3,
            ..Self::file(number)
        }
    }

    /// A freed slot that still carries the `FILE` signature.
    pub fn unused(number: u64) -> Self {
        Self {
            flags: 0,
            ..Self::file(number)
        }
    }

    pub fn extension(number: u64, base: FileRef) -> Self {
        Self {
            base: base.raw(),
            ..Self::file(number)
        }
    }

    pub fn sequence(mut self, sequence: u16) -> Self {
        self.sequence = sequence;
        self
    }

    pub fn named(mut self, parent: FileRef, namespace: u8, name: &str) -> Self {
        let text = utf16_bytes(name);
        let mut value = vec![0u8; 0x42 + text.len()];
        put64(&mut value, 0, parent.raw());
        value[0x40] = (text.len() / 2) as u8;
        value[0x41] = namespace;
        value[0x42..].copy_from_slice(&text);
        self.attributes.push(resident(0x30, "", &value));
        self
    }

    pub fn standard(mut self, attributes: u32) -> Self {
        let mut value = vec![0u8; 0x48];
        put32(&mut value, 0x20, attributes);
        self.attributes.push(resident(0x10, "", &value));
        self
    }

    /// Resident `$DATA` (`stream` empty: unnamed) of `len` patterned bytes.
    pub fn resident_data(mut self, stream: &str, len: usize) -> Self {
        let value: Vec<u8> = (0..len).map(|index| (index % 251) as u8 + 1).collect();
        self.attributes.push(resident(0x80, stream, &value));
        self
    }

    pub fn data(
        mut self,
        stream: &str,
        lowest_vcn: u64,
        allocated: u64,
        size: u64,
        compressed: Option<u64>,
    ) -> Self {
        self.attributes.push(non_resident(
            stream, lowest_vcn, allocated, size, compressed,
        ));
        self
    }

    pub fn reparse(mut self, tag: u32) -> Self {
        let mut value = vec![0u8; 8];
        put32(&mut value, 0, tag);
        self.attributes.push(resident(0xC0, "", &value));
        self
    }

    pub fn list(mut self, segments: &[FileRef]) -> Self {
        let mut value = Vec::new();
        for segment in segments {
            let mut entry = vec![0u8; 0x20];
            put32(&mut entry, 0, 0x80);
            put16(&mut entry, 4, 0x20);
            entry[7] = 0x1A;
            put64(&mut entry, 0x10, segment.raw());
            value.extend_from_slice(&entry);
        }
        self.attributes.push(resident(0x20, "", &value));
        self
    }

    /// The record bytes before the fixup protection.
    pub fn plain(&self) -> Vec<u8> {
        let count = RECORD / 512 + 1;
        let first = align8(USA_OFFSET + 2 * count);
        let mut bytes = vec![0u8; RECORD];
        bytes[0..4].copy_from_slice(b"FILE");
        put16(&mut bytes, 4, USA_OFFSET as u16);
        put16(&mut bytes, 6, count as u16);
        put16(&mut bytes, 0x10, self.sequence);
        put16(&mut bytes, 0x14, first as u16);
        put16(&mut bytes, 0x16, self.flags);
        put32(&mut bytes, 0x1C, RECORD as u32);
        put64(&mut bytes, 0x20, self.base);
        put32(&mut bytes, 0x2C, self.number as u32);
        let mut offset = first;
        for attribute in &self.attributes {
            bytes[offset..offset + attribute.len()].copy_from_slice(attribute);
            offset += attribute.len();
        }
        put32(&mut bytes, offset, 0xFFFF_FFFF);
        put32(&mut bytes, 0x18, (offset + 8) as u32);
        bytes
    }

    /// The record as stored on disk: every 512-byte stride ends in the USN.
    pub fn build(&self) -> Vec<u8> {
        let mut bytes = self.plain();
        let count = RECORD / 512 + 1;
        put16(&mut bytes, USA_OFFSET, USN);
        for sector in 1..count {
            let at = sector * 512 - 2;
            let original = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
            put16(&mut bytes, USA_OFFSET + 2 * sector, original);
            put16(&mut bytes, at, USN);
        }
        bytes
    }
}

/// The volume root (record 5), which names itself ".".
pub(super) fn root() -> Rec {
    Rec::dir(5)
        .sequence(5)
        .standard(0x16)
        .named(reference(5, 5), WIN32_AND_DOS, ".")
}

pub(super) const ROOT: FileRef = FileRef {
    record: 5,
    sequence: 5,
};

pub(super) fn volume(records: usize) -> NtfsVolumeData {
    NtfsVolumeData {
        serial: 0x1122_3344_5566_7788,
        bytes_per_sector: 512,
        bytes_per_cluster: 4096,
        bytes_per_record: RECORD as u32,
        mft_valid_length: (records * RECORD) as u64,
    }
}

/// The whole MFT: each record at its number, empty slots zeroed.
pub(super) fn mft_bytes(records: &[Rec]) -> Vec<u8> {
    let count = records
        .iter()
        .map(|record| record.number as usize + 1)
        .max()
        .unwrap_or(0)
        .max(24);
    let mut bytes = vec![0u8; count * RECORD];
    for record in records {
        let at = record.number as usize * RECORD;
        bytes[at..at + RECORD].copy_from_slice(&record.build());
    }
    bytes
}

/// The index of `records`, fed in spans of `span` bytes.
pub(super) fn index_in_spans(records: &[Rec], span: usize) -> MftIndex {
    index_from(mft_bytes(records), span)
}

/// The index of raw MFT bytes, fed in spans of `span` bytes.
pub(super) fn index_from(mut bytes: Vec<u8>, span: usize) -> MftIndex {
    let mut reader = MftRecordReader::new(&volume(bytes.len() / RECORD)).expect("reader");
    for (index, chunk) in bytes.chunks_mut(span).enumerate() {
        reader
            .feed((index * span) as u64, chunk)
            .expect("spans in order");
    }
    reader.finish()
}

pub(super) fn index(records: &[Rec]) -> MftIndex {
    index_in_spans(records, 64 * RECORD)
}
