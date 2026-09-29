//! Fresh, identity-preserving observations for one logical file name.
use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::vfs::{Backend, VfsMeta};
use super::duplicate_types::FileVariant;
use super::paths::parent_of;
use super::snapshot_hash::md5_hex_to_u64;
use super::types::Sig;

pub(super) fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(io::ErrorKind::Interrupted, "Konfliktauflösung abgebrochen"))
    } else { Ok(()) }
}

pub(super) fn changed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData,
        "Dateivarianten haben sich geändert. Bitte den Vergleich aktualisieren und erneut auswählen.")
}

pub(super) fn metadata(backend: &dyn Backend, path: &str) -> io::Result<Vec<VfsMeta>> {
    let parent = parent_of(path).unwrap_or_default();
    let name = path.rsplit('/').next().unwrap_or(path);
    backend.invalidate_cache();
    match backend.stat(&parent) {
        Ok(meta) if meta.is_dir && !meta.is_symlink => {}
        Ok(_) => return Err(changed()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    }
    let mut ids = HashSet::new();
    let mut entries: Vec<_> = backend.list_dir_for_sync(&parent)?.into_iter()
        .filter(|meta| meta.name == name).collect();
    for meta in &entries {
        if meta.is_dir || meta.is_symlink
            || (entries.len() > 1 && meta.id.as_deref().is_none_or(|id| id.is_empty()))
            || meta.id.as_ref().is_some_and(|id| !ids.insert(id)) {
            return Err(changed());
        }
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(entries)
}

fn same_metadata(a: &VfsMeta, b: &VfsMeta) -> bool {
    a.id == b.id && a.size == b.size && a.mtime_ms == b.mtime_ms
        && a.content_md5 == b.content_md5 && a.is_dir == b.is_dir && a.is_symlink == b.is_symlink
}

pub(super) fn observe(
    backend: &dyn Backend, path: &str, expected: Option<&[VfsMeta]>, cancel: &AtomicBool,
) -> io::Result<Vec<FileVariant>> {
    check_cancel(cancel)?;
    let entries = metadata(backend, path)?;
    if let Some(expected) = expected {
        if entries.len() != expected.len()
            || entries.iter().any(|m| !expected.iter().any(|e| same_metadata(e, m))) {
            return Err(changed());
        }
    }
    let mut variants = Vec::with_capacity(entries.len());
    for meta in &entries {
        check_cancel(cancel)?;
        let (content_size, content_md5) = match meta.content_md5.as_deref() {
            Some(hash) if hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit()) =>
                (meta.size, hash.to_ascii_lowercase()),
            _ => read_content(backend, path, meta.id.as_deref(), &mut io::sink(), cancel, None)?,
        };
        variants.push(FileVariant {
            id: meta.id.clone(), content_size,
            signature: Sig { size: meta.size, mtime_ms: meta.mtime_ms,
                hash: md5_hex_to_u64(&content_md5) },
            content_md5,
        });
    }
    let after = metadata(backend, path)?;
    if after.len() != entries.len() || entries.iter().zip(&after).any(|(a, b)| !same_metadata(a, b)) {
        return Err(changed());
    }
    Ok(variants)
}

pub(super) fn verify(
    backend: &dyn Backend, path: &str, expected: &[FileVariant], cancel: &AtomicBool,
) -> io::Result<()> {
    if observe(backend, path, None, cancel)? == expected { Ok(()) } else { Err(changed()) }
}

pub(super) fn read_content(
    backend: &dyn Backend, path: &str, id: Option<&str>, writer: &mut dyn Write,
    cancel: &AtomicBool,
    throttle: Option<&super::types::Throttle>,
) -> io::Result<(u64, String)> {
    let mut reader = backend.open_read_id(path, id)?;
    let mut hash = md5::Context::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        check_cancel(cancel)?;
        let count = reader.read(&mut buffer)?;
        if count == 0 { break; }
        writer.write_all(&buffer[..count])?;
        hash.consume(&buffer[..count]);
        bytes = bytes.checked_add(count as u64).ok_or_else(changed)?;
        if let Some(throttle) = throttle { throttle.consume(count as u64); }
    }
    Ok((bytes, format!("{:x}", hash.compute())))
}

pub(super) fn verify_content(actual: (u64, String), expected: &FileVariant) -> io::Result<()> {
    if actual == (expected.content_size, expected.content_md5.clone()) { Ok(()) }
    else { Err(changed()) }
}
