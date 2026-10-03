//! Tolerant peer listings; a failed stream never commits a partial directory.
use super::{
    backend::PeerBackend,
    peer_stream,
    wire::{FsListBatch, FsRequest, FsResponse},
};
use crate::vfs::VfsListing;
use std::{collections::HashSet, io, sync::atomic::AtomicBool};

pub(super) fn list(backend: &PeerBackend, path: &str) -> io::Result<VfsListing> {
    if !peer_stream::features(backend, path)?.list_batches_v1 {
        return match backend.request(FsRequest::ListDir { path: path.into() })? {
            FsResponse::Entries { entries } => Ok(VfsListing::complete(
                entries.into_iter().map(Into::into).collect(),
            )),
            _ => Err(peer_stream::invalid("Unerwartete Listen-Antwort")),
        };
    }
    let cancel = AtomicBool::new(false);
    let mut result = VfsListing::default();
    let mut names = HashSet::new();
    let mut previous: Option<String> = None;
    let mut memory = 0usize;
    peer_stream::call(
        backend,
        FsRequest::ListDirBatch(FsListBatch {
            path: path.into(),
            cursor: None,
        }),
        &cancel,
        |response| match response {
            FsResponse::EntriesBatch { entries, omitted } => {
                for entry in entries {
                    crate::vfs::validate_child_name(&entry.name)?;
                    if previous.as_ref().is_some_and(|last| entry.name <= *last)
                        || !names.insert(entry.name.clone())
                    {
                        return Err(peer_stream::invalid(
                            "Unsortierte oder doppelte Listeneinträge",
                        ));
                    }
                    previous = Some(entry.name.clone());
                    memory = memory.saturating_add(
                        entry.name.capacity() * 2
                            + entry.id.as_ref().map_or(0, String::capacity)
                            + std::mem::size_of::<crate::vfs::VfsMeta>() * 2
                            + 3 * std::mem::size_of::<String>(),
                    );
                    result.entries.push(entry.into());
                }
                for hole in omitted {
                    if hole.rel.is_empty()
                        || hole.rel.contains('/')
                        || !names.insert(hole.rel.clone())
                    {
                        return Err(peer_stream::invalid(
                            "Ungültige oder doppelte Listenauslassung",
                        ));
                    }
                    memory = memory.saturating_add(
                        hole.rel.capacity() * 2
                            + hole.detail.capacity()
                            + std::mem::size_of::<crate::vfs::VfsOmission>() * 2
                            + 3 * std::mem::size_of::<String>(),
                    );
                    result.omitted.push(hole.into());
                }
                if memory > crate::transfer::memory_budget() {
                    return Err(io::Error::new(
                        io::ErrorKind::OutOfMemory,
                        "Listen-Ergebnis übersteigt den verfügbaren Empfängerspeicher",
                    ));
                }
                Ok(None)
            }
            FsResponse::EntriesDone { entries, omitted } => {
                if entries != result.entries.len() as u64 || omitted != result.omitted.len() as u64
                {
                    return Err(peer_stream::invalid("Listensumme stimmt nicht"));
                }
                Ok(Some(std::mem::take(&mut result)))
            }
            _ => Err(peer_stream::invalid("Unerwartete Listen-Meldung")),
        },
    )
}
