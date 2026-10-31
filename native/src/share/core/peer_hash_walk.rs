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
    vfs::{HashWalkItem, HashWalkRequest, KeyedWalkItem},
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// Known key prefixes one request carries: its base64 text stays well below
/// the request frame limit (`framing::MAX_REQUEST_CTRL_FRAME`, 256 KiB).
const KNOWN_KEYS: usize = 10 * 1024;

pub(super) fn walk(
    backend: &PeerBackend,
    root: &str,
    request: HashWalkRequest,
    tx: crossbeam_channel::Sender<HashWalkItem>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    let plain = |item: KeyedWalkItem| match item {
        KeyedWalkItem::Item(item) => Ok(item),
        _ => Err(peer_stream::invalid("Unerwarteter Ordnerschlüssel")),
    };
    walk_with(backend, root, request, None, tx, plain, cancel)
}

/// `hash_walk_keys_v1`: the walk with listing attributes and folder keys;
/// `Ok(false)` before sending when the host does not offer it.
pub(super) fn walk_keyed(
    backend: &PeerBackend,
    root: &str,
    request: HashWalkRequest,
    known: &[[u8; 16]],
    tx: crossbeam_channel::Sender<KeyedWalkItem>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    if !request.meta || !peer_stream::features(backend, root)?.hash_walk_keys_v1 {
        return Ok(false);
    }
    // The client puts the keys covering most first.
    let known = &known[..known.len().min(KNOWN_KEYS)];
    walk_with(backend, root, request, Some(known), tx, Ok, cancel)
}

fn walk_with<T: Send>(
    backend: &PeerBackend,
    root: &str,
    request: HashWalkRequest,
    known: Option<&[[u8; 16]]>,
    tx: crossbeam_channel::Sender<T>,
    wrap: impl Fn(KeyedWalkItem) -> io::Result<T>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    let features = peer_stream::features(backend, root)?;
    if !features.hash_walk_v1 || (request.meta && !features.hash_walk_meta_v1) {
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
        single_file: false,
        meta: request.meta,
        keys: known.is_some(),
        known: known.map_or_else(String::new, |known| {
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, known.concat())
        }),
    });
    let result = backend.node.block_on_route(opened.generation, async {
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
                        let regular = entry.is_regular();
                        if !peer_stream::relative(&entry.rel)
                            || entry.meta.is_some() != request.meta
                            || entry.meta.is_some_and(|meta| {
                                entry.is_dir && (meta.is_symlink || meta.special)
                                    || meta.is_symlink && meta.special
                            })
                            || !regular && entry.digest.is_some()
                            || regular && entry.size < request.min_bytes
                        {
                            return Err(peer_stream::invalid("Ungültiger Hash-Walk-Eintrag"));
                        }
                        let digits =
                            request
                                .algorithm
                                .map(|algo| if algo == HashAlgorithm::Md5 { 32 } else { 64 });
                        if regular
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
                        let item = wrap(KeyedWalkItem::Item(entry.into()))?;
                        publish(&tx, cancel, item).await?;
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
                        let item = wrap(KeyedWalkItem::Item(HashWalkItem::Omitted(hole.into())))?;
                        publish(&tx, cancel, item).await?;
                        totals.1 = totals
                            .1
                            .checked_add(1)
                            .ok_or_else(|| peer_stream::invalid("Walk-Zählerüberlauf"))?;
                    }
                }
                FsResponse::HashWalk {
                    message: FsHashWalkMessage::Run { rel },
                } => {
                    if !(rel.is_empty() || peer_stream::relative(&rel)) {
                        return Err(peer_stream::invalid("Ungültiger Ordnerlauf"));
                    }
                    let item = wrap(KeyedWalkItem::Run { rel })?;
                    publish(&tx, cancel, item).await?;
                }
                FsResponse::HashWalk {
                    message: FsHashWalkMessage::Key { rel, key, same },
                } => {
                    let key = folder_key(&key)
                        .filter(|_| rel.is_empty() || peer_stream::relative(&rel))
                        .ok_or_else(|| peer_stream::invalid("Ungültiger Ordnerschlüssel"))?;
                    let item = wrap(KeyedWalkItem::Key { rel, key, same })?;
                    publish(&tx, cancel, item).await?;
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
/// 64 lowercase hex digits as a folder key.
fn folder_key(hex: &str) -> Option<[u8; 32]> {
    if !crate::vfs::is_sha256_hex(hex) {
        return None;
    }
    let mut key = [0u8; 32];
    for (index, slot) in key.iter_mut().enumerate() {
        *slot = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}

async fn publish<T>(
    tx: &crossbeam_channel::Sender<T>,
    cancel: &AtomicBool,
    mut item: T,
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
