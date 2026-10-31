//! Stream host hashes with channel backpressure and cancellation; never return partial success.
//! A host with `hash_walk_filter_v1` gets the request's scope (fresh digests,
//! the job's filter with `meta`); a regular file its filter leaves out then
//! comes without a digest.
use super::{
    backend::PeerBackend,
    framing,
    fs_response::{FsHashWalkMessage, FsWalkFilter, FsWalkScope},
    io_deadline, peer_stream,
    wire::{Ctrl, FsHashAlgo, FsHashWalk, FsRequest, FsResponse},
};
use crate::{
    analytics::HashAlgorithm,
    vfs::{HashWalkItem, HashWalkRequest, KeyedWalk, KeyedWalkItem},
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

/// `hash_walk_keys_v1`: the walk with listing attributes and folder keys
/// as `walk` asks; `Ok(false)` before sending when the host does not offer
/// it. Named folders go in requests of at most `RUN_FOLDERS_BYTES` path
/// text (the request frame holds 256 KiB).
pub(super) fn walk_keyed(
    backend: &PeerBackend,
    root: &str,
    request: HashWalkRequest,
    walk: KeyedWalk<'_>,
    tx: crossbeam_channel::Sender<KeyedWalkItem>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    if !request.meta || !peer_stream::features(backend, root)?.hash_walk_keys_v1 {
        return Ok(false);
    }
    let requests = match walk {
        KeyedWalk::Marks { known } => {
            // The client puts the keys covering most first.
            let known = &known[..known.len().min(KNOWN_KEYS)];
            vec![Keyed {
                known: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    known.concat(),
                ),
                ..Keyed::default()
            }]
        }
        KeyedWalk::KeysOnly => vec![Keyed {
            keys_only: true,
            ..Keyed::default()
        }],
        KeyedWalk::Runs { folders } => {
            let mut portions: Vec<Keyed> = Vec::new();
            let mut bytes = usize::MAX;
            for folder in folders {
                if bytes.saturating_add(folder.len() + 8) > RUN_FOLDERS_BYTES {
                    portions.push(Keyed::default());
                    bytes = 0;
                }
                bytes += folder.len() + 8;
                if let Some(portion) = portions.last_mut() {
                    portion.folders.push(folder.clone());
                }
            }
            portions
        }
    };
    for keyed in requests {
        if !walk_with(
            backend,
            root,
            request.clone(),
            Some(keyed),
            tx.clone(),
            Ok,
            cancel,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Path text of one request naming folders.
const RUN_FOLDERS_BYTES: usize = 160 * 1024;

/// The keyed fields of one request.
#[derive(Default)]
struct Keyed {
    known: String,
    keys_only: bool,
    folders: Vec<String>,
}

fn walk_with<T: Send>(
    backend: &PeerBackend,
    root: &str,
    request: HashWalkRequest,
    keyed: Option<Keyed>,
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
    let scope = features.hash_walk_filter_v1.then(|| FsWalkScope {
        fresh: request.fresh,
        filter: request
            .filter
            .as_deref()
            .filter(|_| request.meta)
            .map(FsWalkFilter::bounded),
    });
    // A file the host's filter leaves out comes without a digest.
    let filtered = scope.as_ref().is_some_and(|scope| scope.filter.is_some());
    let req = FsRequest::HashWalk(FsHashWalk {
        path: root.into(),
        min_bytes: request.min_bytes,
        algo: request.algorithm.map(|algo| match algo {
            HashAlgorithm::Md5 => FsHashAlgo::Md5,
            HashAlgorithm::Sha256 => FsHashAlgo::Sha256,
        }),
        single_file: false,
        meta: request.meta,
        scope,
        keys: keyed.is_some(),
        known: keyed.as_ref().map_or_else(String::new, |k| k.known.clone()),
        keys_only: keyed.as_ref().is_some_and(|k| k.keys_only),
        folders: keyed.map_or_else(Vec::new, |k| k.folders),
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
                                (Some(_), None) => !filtered,
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
                    message:
                        FsHashWalkMessage::Key {
                            rel,
                            key,
                            same,
                            tally,
                        },
                } => {
                    let key = folder_key(&key)
                        .filter(|_| rel.is_empty() || peer_stream::relative(&rel))
                        .ok_or_else(|| peer_stream::invalid("Ungültiger Ordnerschlüssel"))?;
                    let item = wrap(KeyedWalkItem::Key {
                        rel,
                        key,
                        same,
                        tally,
                    })?;
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
