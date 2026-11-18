//! The attributes the size scan needs: `$STANDARD_INFORMATION`,
//! `$ATTRIBUTE_LIST`, `$FILE_NAME`, `$DATA` and `$REPARSE_POINT`.
use super::bytes::{u16_at, u32_at, u64_at, u8_at, utf16_at};
use super::record::{FileName, ParsedRecord};
use super::FileRef;

const STANDARD_INFORMATION: u32 = 0x10;
const ATTRIBUTE_LIST: u32 = 0x20;
const FILE_NAME: u32 = 0x30;
const DATA: u32 = 0x80;
const REPARSE_POINT: u32 = 0xC0;
const FLAG_COMPRESSED: u16 = 0x0001;
const FLAG_SPARSE: u16 = 0x8000;
/// Non-resident header length when it carries the compressed (total
/// allocated) size: compressed or sparse streams.
const NON_RESIDENT_COMPRESSED_HEADER: usize = 0x48;

/// Logical and occupied bytes of one data stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamSize {
    pub size: u64,
    pub allocated: u64,
}

/// The reparse tag, or `Unknown` when the reparse data lives in clusters
/// (non-resident) and the tag is therefore not in the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reparse {
    Tag(u32),
    Unknown,
}

/// The extension segments a resident `$ATTRIBUTE_LIST` names, or
/// `NonResident` when the list itself lives in clusters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttributeList {
    Segments(Vec<FileRef>),
    NonResident,
}

/// The value of a resident attribute; `None` when it is non-resident or
/// does not fit.
fn resident_value(attribute: &[u8]) -> Option<&[u8]> {
    let length = u32_at(attribute, 0x10)? as usize;
    let offset = usize::from(u16_at(attribute, 0x14)?);
    attribute.get(offset..offset.checked_add(length)?)
}

fn non_resident(attribute: &[u8]) -> Option<bool> {
    Some(u8_at(attribute, 8)? != 0)
}

/// Apply one attribute to `record`; `None` when the attribute is malformed.
/// Unknown attribute types are skipped.
pub(super) fn apply(attribute: &[u8], type_code: u32, record: &mut ParsedRecord) -> Option<()> {
    let non_resident = non_resident(attribute)?;
    match type_code {
        STANDARD_INFORMATION if !non_resident => {
            record.attributes = Some(u32_at(resident_value(attribute)?, 0x20)?);
        }
        ATTRIBUTE_LIST => {
            record.attribute_list = Some(if non_resident {
                AttributeList::NonResident
            } else {
                AttributeList::Segments(attribute_list(resident_value(attribute)?)?)
            });
        }
        FILE_NAME if !non_resident => record.names.push(file_name(resident_value(attribute)?)?),
        DATA => data(attribute, non_resident, record)?,
        REPARSE_POINT => {
            record.reparse = Some(if non_resident {
                Reparse::Unknown
            } else {
                Reparse::Tag(u32_at(resident_value(attribute)?, 0)?)
            });
        }
        // A non-resident $STANDARD_INFORMATION or $FILE_NAME breaks the format.
        STANDARD_INFORMATION | FILE_NAME => return None,
        _ => {}
    }
    Some(())
}

fn attribute_list(value: &[u8]) -> Option<Vec<FileRef>> {
    let mut segments = Vec::new();
    let mut offset = 0usize;
    while offset < value.len() {
        let length = usize::from(u16_at(value, offset + 4)?);
        if length < 0x1A || offset.checked_add(length)? > value.len() {
            return None;
        }
        segments.push(FileRef::from_raw(u64_at(value, offset + 0x10)?));
        offset += length;
    }
    Some(segments)
}

fn file_name(value: &[u8]) -> Option<FileName> {
    let length = usize::from(u8_at(value, 0x40)?);
    Some(FileName {
        parent: FileRef::from_raw(u64_at(value, 0)?),
        namespace: u8_at(value, 0x41)?,
        name: utf16_at(value, 0x42, length)?,
    })
}

/// Sizes of a `$DATA` instance; `Ok(None)` for a continuation instance
/// (lowest VCN above 0) whose sizes the first instance already carries.
fn stream_size(attribute: &[u8], non_resident: bool) -> Option<Option<StreamSize>> {
    if !non_resident {
        let size = u32_at(attribute, 0x10)? as u64;
        resident_value(attribute)?;
        // Resident bytes live inside the record; Windows reports them
        // rounded to eight bytes, as WinDirStat does.
        return Some(Some(StreamSize {
            size,
            allocated: (size + 7) & !7,
        }));
    }
    if u64_at(attribute, 0x10)? != 0 {
        return Some(None);
    }
    let flags = u16_at(attribute, 0x0C)?;
    let allocated = u64_at(attribute, 0x28)?;
    let size = u64_at(attribute, 0x30)?;
    let runs = usize::from(u16_at(attribute, 0x20)?);
    let occupied = if flags & (FLAG_COMPRESSED | FLAG_SPARSE) != 0 {
        // Compressed and sparse streams record the clusters really held.
        if runs < NON_RESIDENT_COMPRESSED_HEADER {
            return None;
        }
        u64_at(attribute, 0x40)?
    } else {
        allocated
    };
    Some(Some(StreamSize {
        size,
        allocated: occupied,
    }))
}

fn data(attribute: &[u8], non_resident: bool, record: &mut ParsedRecord) -> Option<()> {
    let named = u8_at(attribute, 9)? != 0;
    let Some(sizes) = stream_size(attribute, non_resident)? else {
        return Some(());
    };
    if named {
        record.streams_allocated = record.streams_allocated.saturating_add(sizes.allocated);
    } else {
        record.data = Some(sizes);
    }
    Some(())
}
