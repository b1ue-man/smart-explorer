//! `hash_file_v1` and `hash_file_md5_v1` client: the host hashes one of its
//! files and sends only the SHA-256 or the sync's MD5. A host without the feature, or one that could hash the file only
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
    Ok(single_file(backend, path, FsHashAlgo::Sha256, cancel)?
        .map(|(sha256, size)| ContentDigest { sha256, size }))
}

/// `hash_file_md5_v1`: the sync's MD5 of one file, formed on the host; a
/// host without the feature yields `Ok(None)` (the caller reads the file).
pub(super) fn md5(
    backend: &PeerBackend,
    path: &str,
    cancel: &AtomicBool,
) -> io::Result<Option<String>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    if !peer_stream::features(backend, path)?.hash_file_md5_v1 {
        return Ok(None);
    }
    Ok(single_file(backend, path, FsHashAlgo::Md5, cancel)?.map(|(md5, _)| md5))
}

/// The digest (`algo`) and size of the one regular file `path` of the host.
fn single_file(
    backend: &PeerBackend,
    path: &str,
    algo: FsHashAlgo,
    cancel: &AtomicBool,
) -> io::Result<Option<(String, u64)>> {
    let valid = |hex: &String| match algo {
        FsHashAlgo::Md5 => crate::vfs::is_md5_hex(hex),
        _ => crate::vfs::is_sha256_hex(hex),
    };
    let request = FsRequest::HashWalk(FsHashWalk {
        path: path.into(),
        algo: Some(algo),
        min_bytes: 0,
        single_file: true,
        meta: false,
        ..Default::default()
    });
    let mut found: Option<(String, u64)> = None;
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
                let digest = entry.digest.filter(valid);
                let (true, Some(digest)) = (root_file, digest) else {
                    return Err(peer_stream::invalid("Ungültige Einzeldatei-Prüfsumme"));
                };
                found = Some((digest, entry.size));
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
