//! Validate and allocate a complete file payload before changing the clipboard.
use std::path::Path;
use windows::{core::Result, Win32::System::{DataExchange::{EmptyClipboard, GetClipboardSequenceNumber}, Ole::CF_HDROP}};
use super::{memory::OwnedGlobal, owner::Clipboard};

pub fn write_files(paths: &[String], effect: u32) -> Result<()> {
    if paths.is_empty() { return Ok(()); }
    publish(paths, effect, None).map(|_| ())
}

/// Publish only while the expected clipboard still owns the same generation.
/// Ok(None) leaves a newer clipboard untouched; Some is the published sequence.
pub fn write_files_if_sequence(paths: &[String], effect: u32, expected: u32) -> Result<Option<u32>> {
    if expected == 0 { return Err(super::invalid("Clipboard sequence is unavailable")); }
    publish(paths, effect, Some(expected))
}

fn publish(paths: &[String], effect: u32, expected: Option<u32>) -> Result<Option<u32>> {
    if paths.is_empty() { return Err(super::invalid("Cannot publish an empty file selection")); }
    if !matches!(effect, super::DROPEFFECT_COPY | super::DROPEFFECT_MOVE) {
        return Err(super::invalid("Unsupported clipboard drop effect"));
    }
    let format = super::preferred_drop_effect_fmt()?;
    let files = OwnedGlobal::from_bytes(&encode_paths(paths)?)?;
    let preferred = OwnedGlobal::from_bytes(&effect.to_le_bytes())?;
    let clipboard = Clipboard::open()?;
    if let Some(expected) = expected {
        let observed = sequence()?;
        if observed != expected {
            clipboard.close()?;
            return Ok(None);
        }
    }
    unsafe { EmptyClipboard()?; }
    // Publish the preference first: if the final file publication fails, no
    // CF_HDROP points at temporary files the caller may dispose on failure.
    preferred.publish(format)?;
    files.publish(CF_HDROP.0 as u32)?;
    let published = sequence()?;
    clipboard.close()?;
    Ok(Some(published))
}

fn sequence() -> Result<u32> {
    let sequence = unsafe { GetClipboardSequenceNumber() };
    if sequence == 0 { Err(super::invalid("Clipboard sequence is unavailable")) } else { Ok(sequence) }
}

fn encode_paths(paths: &[String]) -> Result<Vec<u8>> {
    if paths.len() > super::MAX_CLIPBOARD_PATHS {
        return Err(super::invalid("Clipboard contains more than 1,000,000 paths"));
    }
    let mut bytes = super::header(true);
    for path in paths {
        if path.contains('\0') || !Path::new(path).is_absolute() {
            return Err(super::invalid("Clipboard paths must be absolute and contain no NUL"));
        }
        // Keep Unicode, whitespace and case unchanged; only normalize the
        // separator spelling already accepted by this Windows adapter.
        for unit in path.encode_utf16().map(|unit| if unit == b'/' as u16 { b'\\' as u16 } else { unit }).chain(Some(0)) {
            if bytes.len() > super::MAX_CLIPBOARD_BYTES - 4 {
                return Err(super::invalid("Clipboard paths exceed 64 MiB"));
            }
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&[0, 0]);
    Ok(bytes)
}
