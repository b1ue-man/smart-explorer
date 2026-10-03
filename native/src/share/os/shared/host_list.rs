//! Tolerant, sorted folder listings in bounded portions.
use std::{io, sync::atomic::{AtomicBool, Ordering}};
use tokio::sync::mpsc;
use crate::share::{fs_access::FsAccess, fs_response::{FsOmission, LIST_BATCH_MAX_BYTES, LIST_BATCH_MAX_ENTRIES}, host_stream,
    wire::{FsListBatch, FsMeta, FsResponse}};

pub(in crate::share) fn hidden(name: &str) -> bool {
    crate::apptrash::excluded_name(name) || crate::bisync::is_engine_name(name) || crate::vfs::is_staging_name(name)
}
pub(in crate::share) fn listing(path: &str, access: &FsAccess) -> io::Result<(Vec<FsMeta>, Vec<FsOmission>)> {
    let parts = crate::share::fs::split_clean(path)?;
    if access.is_dynamic() && (parts.is_empty() || parts == ["Verbindungen"]) {
        return Ok((access.list_dir(path)?, Vec::new()));
    }
    let target = access.resolve(path)?;
    if target.backend.scheme() == crate::vfs::Scheme::Local
        && crate::share::storage_roots::is_private(&crate::share::storage_roots::local_path(&target)?) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,"Interner Host-Speicher ist geschützt"));
    }
    let listing = crate::vfs::list_dir_tolerant(&*target.backend, &target.path)?;
    let excluded = crate::share::storage_roots::excluded();
    let entries = listing.entries.into_iter().filter(|entry| {
        !hidden(&entry.name) && (target.backend.scheme() != crate::vfs::Scheme::Local || !excluded.iter()
            .any(|hidden| hidden == &std::path::Path::new(&target.path).join(&entry.name)))
    }).map(Into::into).collect();
    let omitted = listing.omitted.into_iter().map(|mut hole| {
        hole.detail = hole.detail.replace(&target.path, path);
        FsOmission::from(hole)
    }).collect();
    Ok((entries, omitted))
}
pub(in crate::share) fn run(request: FsListBatch, access: FsAccess, tx: mpsc::Sender<io::Result<FsResponse>>, cancel: &AtomicBool) -> io::Result<()> {
    if let Some(cursor) = &request.cursor { crate::vfs::validate_child_name(cursor)?; }
    let (mut entries, mut omitted) = listing(&request.path, &access)?;
    entries.sort_by(|a,b| a.name.cmp(&b.name)); omitted.sort_by(|a,b| a.rel.cmp(&b.rel));
    let after = |name: &str| request.cursor.as_ref().is_none_or(|cursor| name > cursor.as_str());
    entries.retain(|entry| after(&entry.name)); omitted.retain(|entry| after(&entry.rel));
    let totals = (entries.len() as u64, omitted.len() as u64);
    let mut batch = Vec::new(); let mut holes = Vec::new(); let mut bytes = 0;
    for entry in entries {
        if cancel.load(Ordering::Relaxed) { return Err(io::ErrorKind::Interrupted.into()); }
        let cost = serde_json::to_vec(&entry).map_err(io::Error::other)?.len();
        if cost > LIST_BATCH_MAX_BYTES { return Err(io::Error::new(io::ErrorKind::InvalidData, "Metadaten überschreiten das Listenformat")); }
        if !batch.is_empty() && (batch.len() >= LIST_BATCH_MAX_ENTRIES || bytes + cost > LIST_BATCH_MAX_BYTES) {
            host_stream::emit(&tx, cancel, FsResponse::EntriesBatch { entries: std::mem::take(&mut batch), omitted: Vec::new() })?; bytes = 0;
        }
        bytes += cost; batch.push(entry);
    }
    if !batch.is_empty() { host_stream::emit(&tx, cancel, FsResponse::EntriesBatch { entries: batch, omitted: Vec::new() })?; }
    bytes = 0;
    for hole in omitted {
        let cost = serde_json::to_vec(&hole).map_err(io::Error::other)?.len();
        if cost > LIST_BATCH_MAX_BYTES { return Err(io::Error::new(io::ErrorKind::InvalidData, "Auslassung überschreitet das Listenformat")); }
        if !holes.is_empty() && (holes.len() >= LIST_BATCH_MAX_ENTRIES || bytes + cost > LIST_BATCH_MAX_BYTES) {
            host_stream::emit(&tx, cancel, FsResponse::EntriesBatch { entries: Vec::new(), omitted: std::mem::take(&mut holes) })?; bytes = 0;
        }
        bytes += cost; holes.push(hole);
    }
    if !holes.is_empty() { host_stream::emit(&tx, cancel, FsResponse::EntriesBatch { entries: Vec::new(), omitted: holes })?; }
    host_stream::emit(&tx, cancel, FsResponse::EntriesDone { entries: totals.0, omitted: totals.1 })
}
