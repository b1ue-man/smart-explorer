//! Bounds-checked decoding of variable-length Windows directory records.
use std::{ffi::OsString, io, mem::offset_of, os::windows::ffi::OsStringExt};
use windows_sys::Win32::Storage::FileSystem::{FILE_FULL_DIR_INFO, FILE_ID_EXTD_DIR_INFO};

#[derive(Clone, Copy)]
pub(super) enum Layout { Extended, Full }

pub(super) struct Record {
    pub name: OsString,
    pub size: u64,
    pub attributes: u32,
    pub tag: Option<u32>,
    pub next: Option<usize>,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Ungültiger Windows-Verzeichniseintrag")
}

fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let value = bytes.get(offset..offset + 4).ok_or_else(invalid)?;
    Ok(u32::from_le_bytes(value.try_into().map_err(|_| invalid())?))
}

pub(super) fn decode(bytes: &[u8], offset: usize, layout: Layout) -> io::Result<Record> {
    let bytes = bytes.get(offset..).ok_or_else(invalid)?;
    let name_offset = match layout {
        Layout::Extended => offset_of!(FILE_ID_EXTD_DIR_INFO, FileName),
        Layout::Full => offset_of!(FILE_FULL_DIR_INFO, FileName),
    };
    let next = u32_at(bytes, 0)? as usize;
    let name_len = u32_at(bytes, offset_of!(FILE_FULL_DIR_INFO, FileNameLength))? as usize;
    if name_len == 0 || name_len % 2 != 0 { return Err(invalid()); }
    let end = name_offset.checked_add(name_len).ok_or_else(invalid)?;
    if next != 0 && (next % 8 != 0 || next < end || next >= bytes.len()) {
        return Err(invalid());
    }
    let units: Vec<u16> = bytes.get(name_offset..end).ok_or_else(invalid)?
        .chunks_exact(2).map(|v| u16::from_le_bytes([v[0], v[1]])).collect();
    if units.iter().any(|v| matches!(*v, 0 | 47 | 92)) { return Err(invalid()); }
    let size_offset = offset_of!(FILE_FULL_DIR_INFO, EndOfFile);
    let size = i64::from_le_bytes(bytes.get(size_offset..size_offset + 8)
        .ok_or_else(invalid)?.try_into().map_err(|_| invalid())?);
    if size < 0 { return Err(invalid()); }
    Ok(Record {
        name: OsString::from_wide(&units),
        size: size as u64,
        attributes: u32_at(bytes, offset_of!(FILE_FULL_DIR_INFO, FileAttributes))?,
        tag: match layout {
            Layout::Extended => Some(u32_at(bytes, offset_of!(FILE_ID_EXTD_DIR_INFO, ReparsePointTag))?),
            Layout::Full => None,
        },
        next: if next == 0 { None } else { Some(offset.checked_add(next).ok_or_else(invalid)?) },
    })
}
