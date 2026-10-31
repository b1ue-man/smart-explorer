//! `WalkKeyed` (`walk-keys-v1`) for the backend the service serves: relayed
//! to the backend's own keyed walk next to its data (a Share host with
//! `hash_walk_keys_v1`), runs as `HashPart`s and every folder key as a
//! `FolderKey` frame in the order the host sent them. A backend without one
//! answers "unsupported", so the client takes the walk without keys.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::agent::ext_wire::algorithm_from_wire;
use crate::agent_proto::{Frame, UNSUPPORTED_EXTENSION};
use crate::vfs::{self as vfs, BackendHandle, HashWalkRequest, KeyedWalkItem};

use super::super::backend_server::emit;
use super::{canceled, Items, Sink};

pub(super) fn serve(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    (algorithm, min_bytes, known): (u8, u64, &[u8]),
    cancel: &AtomicBool,
) -> io::Result<()> {
    if known.len() % 16 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "known folder keys are no whole 16-byte prefixes",
        ));
    }
    let known: Vec<[u8; 16]> = known
        .chunks_exact(16)
        .map(|chunk| {
            let mut prefix = [0u8; 16];
            prefix.copy_from_slice(chunk);
            prefix
        })
        .collect();
    let request = HashWalkRequest {
        algorithm: algorithm_from_wire(algorithm),
        min_bytes,
        meta: true,
    };
    let mut items = Items::new(sink, id, true);
    let (tx, rx) = crossbeam_channel::bounded::<KeyedWalkItem>(1024);
    let stop = AtomicBool::new(false);
    let mut forwarded = Ok(());
    let walked = std::thread::scope(|scope| {
        let walker =
            scope.spawn(|| vfs::hash_walk_keyed(&**backend, root, request, &known, tx, &stop));
        loop {
            if cancel.load(Ordering::Relaxed) {
                stop.store(true, Ordering::Relaxed);
            }
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(item) if forwarded.is_ok() => {
                    forwarded = forward(&mut items, item);
                    if forwarded.is_err() {
                        stop.store(true, Ordering::Relaxed);
                    }
                }
                Ok(_) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
        walker.join()
    });
    forwarded?;
    match walked {
        Ok(Ok(true)) => {
            items.flush()?;
            emit(sink, id, &Frame::End)
        }
        Ok(Ok(false)) => emit(sink, id, &Frame::Err(UNSUPPORTED_EXTENSION.to_string())),
        Ok(Err(_)) if cancel.load(Ordering::Relaxed) => Err(canceled()),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(io::Error::other("keyed hash walk of the peer stopped")),
    }
}

/// One relayed item; a mark goes out after every item before it.
fn forward(items: &mut Items<'_>, item: KeyedWalkItem) -> io::Result<()> {
    match item {
        KeyedWalkItem::Item(item) => items.send(item),
        KeyedWalkItem::Run { rel } => {
            items.flush()?;
            emit(items.sink, items.id, &Frame::FolderRun(rel))
        }
        KeyedWalkItem::Key { rel, key, same } => {
            items.flush()?;
            emit(items.sink, items.id, &Frame::FolderKey { rel, key, same })
        }
    }
}
