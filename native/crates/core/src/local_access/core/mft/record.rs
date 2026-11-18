//! One MFT file record segment: signature, update sequence (fixup) and the
//! attribute chain, parsed from bytes into `ParsedRecord`.
use super::attributes::{self, AttributeList, Reparse, StreamSize};
use super::bytes::{u16_at, u32_at, u64_at};
use super::FileRef;
use std::fmt;

const SIGNATURE_FILE: u32 = u32::from_le_bytes(*b"FILE");
const SIGNATURE_BAAD: u32 = u32::from_le_bytes(*b"BAAD");
/// The update sequence protects every 512-byte stride of a record,
/// whatever the volume's sector size.
const FIXUP_STRIDE: usize = 512;
const FLAG_IN_USE: u16 = 0x0001;
const FLAG_DIRECTORY: u16 = 0x0002;
/// Smallest header that holds the base reference (offset 0x20..0x28).
const HEADER_LEN: usize = 0x28;
const ATTRIBUTE_END: u32 = 0xFFFF_FFFF;

/// Why an in-use record could not be read; it makes the scan partial.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordDefect {
    /// Neither `FILE` nor an empty slot (`BAAD` marks a failed multi-sector write).
    Signature(u32),
    /// The update sequence array does not fit the record.
    UpdateSequence,
    /// A sector's last word differs from the update sequence number: a torn write.
    Fixup { sector: usize },
    /// Header offsets point outside the record.
    Header,
    /// The attribute at this offset is truncated or inconsistent.
    Attribute { offset: usize, type_code: u32 },
    /// The attribute chain ends without its end marker.
    MissingEnd,
}

impl fmt::Display for RecordDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signature(SIGNATURE_BAAD) => f.write_str("als defekt markiert (BAAD)"),
            Self::Signature(value) => write!(f, "unbekannte Signatur {value:#010x}"),
            Self::UpdateSequence => f.write_str("Update-Sequence-Array passt nicht zum Datensatz"),
            Self::Fixup { sector } => write!(f, "Fixup-Prüfung in Sektor {sector} fehlgeschlagen"),
            Self::Header => f.write_str("Kopfdaten zeigen aus dem Datensatz hinaus"),
            Self::Attribute { offset, type_code } => {
                write!(
                    f,
                    "Attribut {type_code:#x} bei Offset {offset:#x} ist ungültig"
                )
            }
            Self::MissingEnd => f.write_str("Attributkette ohne Endmarke"),
        }
    }
}

/// One `$FILE_NAME` of a record (8.3 DOS names are dropped by the index).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileName {
    pub parent: FileRef,
    /// 0 POSIX, 1 Win32, 2 DOS, 3 Win32 and DOS.
    pub namespace: u8,
    pub name: Vec<u16>,
}

/// The facts one record segment contributes to its file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedRecord {
    pub number: u64,
    pub sequence: u16,
    /// `Some` for an extension segment: the base record it belongs to.
    pub base: Option<FileRef>,
    pub is_directory: bool,
    /// `$STANDARD_INFORMATION` file attributes (`FILE_ATTRIBUTE_*`).
    pub attributes: Option<u32>,
    pub names: Vec<FileName>,
    /// Unnamed `$DATA` (only the instance starting at VCN 0 carries sizes).
    pub data: Option<StreamSize>,
    /// Occupied bytes of all named `$DATA` streams (alternate data streams,
    /// including `WofCompressedData`) starting at VCN 0 in this segment.
    pub streams_allocated: u64,
    pub reparse: Option<Reparse>,
    pub attribute_list: Option<AttributeList>,
}

/// An empty or freed slot, or a file record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordState {
    Unused,
    InUse(ParsedRecord),
}

/// Apply the fixup to `bytes` (one whole record segment) in place and parse
/// it. A slot that is not in use is `Unused` even when torn: its content
/// does not count.
pub fn parse_record(bytes: &mut [u8], number: u64) -> Result<RecordState, RecordDefect> {
    let signature = u32_at(bytes, 0).ok_or(RecordDefect::Header)?;
    if signature == 0 {
        return Ok(RecordState::Unused);
    }
    if signature != SIGNATURE_FILE {
        return Err(RecordDefect::Signature(signature));
    }
    if bytes.len() < HEADER_LEN || !bytes.len().is_multiple_of(FIXUP_STRIDE) {
        return Err(RecordDefect::Header);
    }
    // The flags lie inside the first stride, before its protected last word.
    let flags = u16_at(bytes, 0x16).ok_or(RecordDefect::Header)?;
    if flags & FLAG_IN_USE == 0 {
        return Ok(RecordState::Unused);
    }
    apply_fixup(bytes)?;
    let first_attribute = usize::from(u16_at(bytes, 0x14).ok_or(RecordDefect::Header)?);
    let in_use_bytes = u32_at(bytes, 0x18).ok_or(RecordDefect::Header)? as usize;
    if first_attribute < HEADER_LEN || first_attribute >= bytes.len() || in_use_bytes > bytes.len()
    {
        return Err(RecordDefect::Header);
    }
    let base_raw = u64_at(bytes, 0x20).ok_or(RecordDefect::Header)?;
    let base = FileRef::from_raw(base_raw);
    let mut record = ParsedRecord {
        number,
        sequence: u16_at(bytes, 0x10).ok_or(RecordDefect::Header)?,
        base: (base.record != 0).then_some(base),
        is_directory: flags & FLAG_DIRECTORY != 0,
        ..ParsedRecord::default()
    };
    parse_attributes(
        &bytes[..in_use_bytes.max(first_attribute)],
        first_attribute,
        &mut record,
    )?;
    Ok(RecordState::InUse(record))
}

fn apply_fixup(bytes: &mut [u8]) -> Result<(), RecordDefect> {
    let offset = usize::from(u16_at(bytes, 0x04).ok_or(RecordDefect::UpdateSequence)?);
    let count = usize::from(u16_at(bytes, 0x06).ok_or(RecordDefect::UpdateSequence)?);
    let strides = bytes.len() / FIXUP_STRIDE;
    let end = offset
        .checked_add(count * 2)
        .ok_or(RecordDefect::UpdateSequence)?;
    if count != strides + 1
        || offset < HEADER_LEN
        || !offset.is_multiple_of(2)
        || end > FIXUP_STRIDE - 2
    {
        return Err(RecordDefect::UpdateSequence);
    }
    let array: Vec<u16> = (0..count)
        .map(|index| u16_at(bytes, offset + index * 2))
        .collect::<Option<_>>()
        .ok_or(RecordDefect::UpdateSequence)?;
    let usn = array[0];
    for (sector, replacement) in array.iter().enumerate().skip(1) {
        let at = sector * FIXUP_STRIDE - 2;
        if u16_at(bytes, at) != Some(usn) {
            return Err(RecordDefect::Fixup { sector: sector - 1 });
        }
        bytes[at..at + 2].copy_from_slice(&replacement.to_le_bytes());
    }
    Ok(())
}

fn parse_attributes(
    bytes: &[u8],
    mut offset: usize,
    record: &mut ParsedRecord,
) -> Result<(), RecordDefect> {
    loop {
        let type_code = u32_at(bytes, offset).ok_or(RecordDefect::MissingEnd)?;
        if type_code == ATTRIBUTE_END {
            return Ok(());
        }
        let defect = RecordDefect::Attribute { offset, type_code };
        let length = u32_at(bytes, offset + 4).ok_or(defect.clone())? as usize;
        let end = offset.checked_add(length).ok_or(defect.clone())?;
        if length < 0x18 || !length.is_multiple_of(8) || end > bytes.len() {
            return Err(defect);
        }
        let attribute = &bytes[offset..end];
        attributes::apply(attribute, type_code, record).ok_or(defect)?;
        offset = end;
    }
}
