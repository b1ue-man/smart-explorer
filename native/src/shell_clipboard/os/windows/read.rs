//! Bounded DROPFILES parsing; ANSI conversion is delegated to Windows.
use std::path::Path;
use windows::{core::Result, Win32::{
    Foundation::HGLOBAL,
    System::{DataExchange::{GetClipboardData, IsClipboardFormatAvailable}, Ole::CF_HDROP},
    UI::Shell::{DragQueryFileW, DROPFILES, HDROP},
}};
use super::{memory::{LockedGlobal, OwnedGlobal}, owner::Clipboard};

/// No CF_HDROP is Ok(None); clipboard contention, malformed data, conversion
/// failures and invalid effect payloads are errors, not an empty clipboard.
pub fn read_files() -> Result<Option<(Vec<String>, bool)>> {
    let clipboard = Clipboard::open()?;
    if unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32) }.is_err() {
        clipboard.close()?;
        return Ok(None);
    }
    let handle = unsafe { GetClipboardData(CF_HDROP.0 as u32)? };
    let paths = {
        let locked = unsafe { LockedGlobal::new(HGLOBAL(handle.0))? };
        decode_paths(locked.bytes())?
    };
    let format = super::preferred_drop_effect_fmt()?;
    let is_cut = if unsafe { IsClipboardFormatAvailable(format) }.is_ok() {
        let effect = unsafe { GetClipboardData(format)? };
        let locked = unsafe { LockedGlobal::new(HGLOBAL(effect.0))? };
        decode_effect(locked.bytes())?
    } else { false };
    clipboard.close()?;
    Ok(Some((paths, is_cut)))
}

fn decode_effect(bytes: &[u8]) -> Result<bool> {
    let value: [u8; 4] = bytes.get(..4).ok_or_else(|| super::invalid("Truncated clipboard drop effect"))?
        .try_into().map_err(|_| super::invalid("Invalid clipboard drop effect"))?;
    Ok(u32::from_le_bytes(value) & super::DROPEFFECT_MOVE != 0)
}

fn decode_paths(bytes: &[u8]) -> Result<Vec<String>> {
    if bytes.len() < super::HEADER_BYTES || bytes.len() > super::MAX_CLIPBOARD_BYTES {
        return Err(super::invalid("Invalid DROPFILES allocation size"));
    }
    let header = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<DROPFILES>()) };
    let flag = header.fWide;
    let wide = flag.as_bool();
    let width = if wide { 2 } else { 1 };
    let offset = header.pFiles as usize;
    if offset < super::HEADER_BYTES || offset >= bytes.len() {
        return Err(super::invalid("DROPFILES path offset is outside its payload"));
    }
    let data = &bytes[offset..];
    let mut paths = Vec::new();
    let mut cursor = 0;
    loop {
        let start = cursor;
        while !is_zero(data, cursor, width)? { cursor += width; }
        if cursor == start {
            // A nonempty list already consumed its last path's terminator.
            // An initially empty list must still provide both terminators.
            if paths.is_empty() && !is_zero(data, cursor + width, width)? {
                return Err(super::invalid("DROPFILES list is not double-NUL terminated"));
            }
            return Ok(paths);
        }
        if paths.len() >= super::MAX_CLIPBOARD_PATHS {
            return Err(super::invalid("Clipboard contains more than 1,000,000 paths"));
        }
        let path = if wide {
            let units: Vec<u16> = data[start..cursor].chunks_exact(2)
                .map(|unit| u16::from_le_bytes([unit[0], unit[1]])).collect();
            strict_unicode(&units)?
        } else { decode_ansi(&data[start..cursor])? };
        if !Path::new(&path).is_absolute() {
            return Err(super::invalid("DROPFILES contains a non-absolute path"));
        }
        paths.push(path);
        cursor += width;
    }
}

fn is_zero(bytes: &[u8], offset: usize, width: usize) -> Result<bool> {
    let unit = bytes.get(offset..offset + width)
        .ok_or_else(|| super::invalid("DROPFILES path list is truncated or unterminated"))?;
    Ok(unit.iter().all(|byte| *byte == 0))
}

fn strict_unicode(units: &[u16]) -> Result<String> {
    String::from_utf16(units)
        .map_err(|_| super::invalid("Clipboard filename contains unpaired UTF-16 surrogates"))
}

fn decode_ansi(path: &[u8]) -> Result<String> {
    // Build one validated entry for Windows' ANSI-to-Unicode conversion. Using
    // index zero avoids repeatedly rescanning N preceding paths (quadratic for
    // large legacy CF_HDROP lists). No clipboard-owned memory is modified/freed.
    let mut bytes = super::header(false);
    bytes.extend_from_slice(path);
    bytes.extend_from_slice(&[0, 0]);
    let owned = OwnedGlobal::from_bytes(&bytes)?;
    let drop = HDROP(owned.handle().0);
    let length = unsafe { DragQueryFileW(drop, 0, None) } as usize;
    if length == 0 || length > super::MAX_CLIPBOARD_BYTES / 2 {
        return Err(super::invalid("Windows could not decode the ANSI clipboard filename"));
    }
    let mut units = vec![0; length + 1];
    let copied = unsafe { DragQueryFileW(drop, 0, Some(&mut units)) } as usize;
    if copied != length || units[length] != 0 {
        return Err(super::invalid("Windows returned an incomplete clipboard filename"));
    }
    strict_unicode(&units[..length])
}
