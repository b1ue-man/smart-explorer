//! `hash_file_v1` client: the host hashes one of its files and sends only the
//! SHA-256. A host without the feature, or one that could hash the file only
//! by downloading it itself, yields `Ok(None)`; any other answer than exactly
//! one root entry with a valid digest is an error.
use super::{
    backend::PeerBackend,
    fs_response::FsHashWalkMessage,
    peer_stream,
    wire::{FsHashAlgo, FsHashWalk, FsRequest, FsResponse},
};
use crate::vfs::ContentDigest;
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn digest(
    backend: &PeerBackend,
    path: &str,
    cancel: &AtomicBool,
) -> io::Result<Option<ContentDigest>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    if !peer_stream::features(backend, path)?.hash_file_v1 {
        return Ok(None);
    }
    let request = FsRequest::HashWalk(FsHashWalk {
        path: path.into(),
        algo: Some(FsHashAlgo::Sha256),
        min_bytes: 0,
        single_file: true,
        meta: false,
    });
    let mut found: Option<ContentDigest> = None;
    peer_stream::call(backend, request, cancel, |response| match response {
        FsResponse::HashWalk {
            message: FsHashWalkMessage::Progress { .. },
        } => Ok(None),
        FsResponse::HashWalk {
            message: FsHashWalkMessage::Batch { entries, omitted },
        } => {
            if let Some(hole) = omitted.first() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Datei nicht prüfbar: {}", hole.detail),
                ));
            }
            for entry in entries {
                let root_file = found.is_none() && entry.rel.is_empty() && !entry.is_dir;
                let sha256 = entry.digest.filter(|hex| crate::vfs::is_sha256_hex(hex));
                let (true, Some(sha256)) = (root_file, sha256) else {
                    return Err(peer_stream::invalid("Ungültige Einzeldatei-Prüfsumme"));
                };
                found = Some(ContentDigest {
                    sha256,
                    size: entry.size,
                });
            }
            Ok(None)
        }
        FsResponse::HashWalk {
            message: FsHashWalkMessage::Done { entries, omitted },
        } => {
            if omitted != 0 || entries != u64::from(found.is_some()) {
                return Err(peer_stream::invalid("Einzeldatei-Prüfsumme unvollständig"));
            }
            Ok(Some(found.take()))
        }
        _ => Err(peer_stream::invalid("Unerwartete Hash-Walk-Meldung")),
    })
}
