//! Stream host hashes with channel backpressure and cancellation; never return partial success.
use super::{
    backend::PeerBackend,
    framing,
    fs_response::FsHashWalkMessage,
    io_deadline, peer_stream,
    wire::{Ctrl, FsHashAlgo, FsHashWalk, FsRequest, FsResponse},
};
use crate::{
    analytics::HashAlgorithm,
    vfs::{HashWalkItem, HashWalkRequest},
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(super) fn walk(
    backend: &PeerBackend,
    root: &str,
    request: HashWalkRequest,
    tx: crossbeam_channel::Sender<HashWalkItem>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    if !peer_stream::features(backend, root)?.hash_walk_v1 {
        return Ok(false);
    }
    let endpoint = backend.current_endpoint()?;
    let mut opened = backend.node.open_stream(&endpoint, &backend.identity)?;
    let lease = backend.mount_lease_token()?;
    let req = FsRequest::HashWalk(FsHashWalk {
        path: root.into(),
        min_bytes: request.min_bytes,
        algo: request.algorithm.map(|algo| match algo {
            HashAlgorithm::Md5 => FsHashAlgo::Md5,
            HashAlgorithm::Sha256 => FsHashAlgo::Sha256,
        }),
    });
    let result = backend.node.block_on(async {
        io_deadline::run(
            "Share hash walk request",
            framing::send_ctrl(&mut opened.send, &Ctrl::Fs { req, lease }),
        )
        .await?;
        let mut totals = (0u64, 0u64);
        loop {
            match peer_stream::response(&mut opened.recv, cancel).await? {
                FsResponse::HashWalk {
                    message: FsHashWalkMessage::Progress { .. },
                } => {}
                FsResponse::HashWalk {
                    message: FsHashWalkMessage::Batch { entries, omitted },
                } => {
                    for entry in entries {
                        if !peer_stream::relative(&entry.rel)
                            || entry.is_dir && entry.digest.is_some()
                            || !entry.is_dir && entry.size < request.min_bytes
                        {
                            return Err(peer_stream::invalid("Ungültiger Hash-Walk-Eintrag"));
                        }
                        let digits =
                            request
                                .algorithm
                                .map(|algo| if algo == HashAlgorithm::Md5 { 32 } else { 64 });
                        if !entry.is_dir
                            && match (digits, entry.digest.as_ref()) {
                                (None, None) => false,
                                (Some(n), Some(hash)) => {
                                    hash.len() != n
                                        || !hash.bytes().all(|b| {
                                            b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
                                        })
                                }
                                _ => true,
                            }
                        {
                            return Err(peer_stream::invalid(
                                "Hash des angeforderten Algorithmus fehlt oder ist ungültig",
                            ));
                        }
                        publish(&tx, cancel, entry.into()).await?;
                        totals.0 = totals
                            .0
                            .checked_add(1)
                            .ok_or_else(|| peer_stream::invalid("Walk-Zählerüberlauf"))?;
                    }
                    for hole in omitted {
                        // Empty identifies the whole root; unrepresentable literal
                        // names remain omissions even when they are no valid locator.
                        if hole.rel.starts_with('/')
                            || hole.rel.split('/').any(|p| p == ".." || p == ".")
                        {
                            return Err(peer_stream::invalid(
                                "Auslassung außerhalb der Walk-Wurzel",
                            ));
                        }
                        publish(&tx, cancel, HashWalkItem::Omitted(hole.into())).await?;
                        totals.1 = totals
                            .1
                            .checked_add(1)
                            .ok_or_else(|| peer_stream::invalid("Walk-Zählerüberlauf"))?;
                    }
                }
                FsResponse::HashWalk {
                    message: FsHashWalkMessage::Done { entries, omitted },
                } => {
                    if totals != (entries, omitted) {
                        return Err(peer_stream::invalid("Hash-Walk-Summe stimmt nicht"));
                    }
                    return Ok(true);
                }
                _ => return Err(peer_stream::invalid("Unerwartete Hash-Walk-Meldung")),
            }
        }
    });
    if let Err(error) = &result {
        peer_stream::abort(
            &mut opened.send,
            &mut opened.recv,
            cancel.load(Ordering::Relaxed),
        );
        if peer_stream::transport(error) {
            let _ = backend
                .node
                .invalidate_outgoing_session(&opened.session_key, opened.generation);
        }
    }
    result
}
async fn publish(
    tx: &crossbeam_channel::Sender<HashWalkItem>,
    cancel: &AtomicBool,
    mut item: HashWalkItem,
) -> io::Result<()> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        match tx.try_send(item) {
            Ok(()) => return Ok(()),
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                return Err(io::ErrorKind::Interrupted.into())
            }
            Err(crossbeam_channel::TrySendError::Full(pending)) => item = pending,
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
