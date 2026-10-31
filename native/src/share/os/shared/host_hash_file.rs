//! `hash_file_v1`: the SHA-256 of one regular file of an export, computed on
//! this host so only the digest crosses the connection. A local export is
//! read from held handles below its root; an exported connection answers
//! only when its own backend hashes next to the data (`vfs::content_digest`),
//! otherwise with no entry, so this host never downloads the file for it.
//! A missing, changed or non-regular file ends the stream with an error,
//! never with a partial answer.
use crate::{
    analytics::Progress,
    share::{
        fs::ResolvedTarget,
        fs_access::FsAccess,
        fs_response::{FsHashEntry, FsHashWalkMessage},
        host_stream, storage_roots,
        wire::{FsHashAlgo, FsHashWalk, FsResponse},
    },
    vfs::{ContentDigest, Scheme},
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::mpsc;

pub(in crate::share) fn run(
    request: FsHashWalk,
    access: FsAccess,
    tx: mpsc::Sender<io::Result<FsResponse>>,
    cancel: Arc<AtomicBool>,
    progress: Progress,
) -> io::Result<()> {
    if request.algo != Some(FsHashAlgo::Sha256) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Einzeldatei-Prüfsumme gibt es nur als SHA-256",
        ));
    }
    let parts = crate::share::fs::split_clean(&request.path)?;
    if parts.is_empty() || access.is_dynamic() && parts == ["Verbindungen"] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Der Pfad ist keine Datei",
        ));
    }
    let target = access.resolve(&request.path)?;
    let entry = if target.backend.scheme() == Scheme::Local {
        Some(local(&target, &cancel, &progress)?)
    } else {
        let digest = crate::vfs::content_digest(&*target.backend, &target.path, &cancel)?;
        digest.map(remote_entry)
    };
    let count = u64::from(entry.is_some());
    if let Some(entry) = entry {
        progress.files.fetch_add(1, Ordering::Relaxed);
        host_stream::emit(
            &tx,
            &cancel,
            FsResponse::HashWalk {
                message: FsHashWalkMessage::Batch {
                    entries: vec![entry],
                    omitted: Vec::new(),
                },
            },
        )?;
    }
    host_stream::emit(
        &tx,
        &cancel,
        FsResponse::HashWalk {
            message: FsHashWalkMessage::Done {
                entries: count,
                omitted: 0,
            },
        },
    )
}

/// An exported connection that hashes next to its own data.
fn remote_entry(digest: ContentDigest) -> FsHashEntry {
    FsHashEntry {
        rel: String::new(),
        is_dir: false,
        size: digest.size,
        mtime_ms: 0,
        digest: Some(digest.sha256),
        meta: None,
    }
}

fn local(
    target: &ResolvedTarget,
    cancel: &AtomicBool,
    progress: &Progress,
) -> io::Result<FsHashEntry> {
    let physical = storage_roots::local_path(target)?;
    if storage_roots::is_private(&physical) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Interner Host-Speicher ist geschützt",
        ));
    }
    let file = storage_roots::open_local_file(target)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Kein regulärer Datenstrom",
        ));
    }
    let mut reader = &file;
    let mut count = |bytes: u64| {
        progress.bytes.fetch_add(bytes, Ordering::Relaxed);
    };
    let sha256 = crate::vfs::sha256_exact(&mut reader, before.len(), cancel, &mut count)?;
    let after = file.metadata()?;
    if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Datei beim Hashen geändert",
        ));
    }
    Ok(FsHashEntry {
        rel: String::new(),
        is_dir: false,
        size: before.len(),
        mtime_ms: before
            .modified()
            .ok()
            .map(crate::local_access::system_time_ms)
            .unwrap_or(0),
        digest: Some(sha256),
        meta: None,
    })
}
