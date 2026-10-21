//! Download packets: many small files read on one pool channel. A member
//! opens its file, puts every READ of `[0, size)` and one more READ just
//! behind the end on the wire at once (the growth probe), then closes the
//! handle; the chains of up to `open-handles` members overlap. The files go
//! to the sink strictly in request order, a later one that is ready first
//! waits in memory (the engine reserved the packet's bytes).
use super::backend::SftpBackend;
use super::batch_ops::{channel_or_unsupported, window, BatchWire, ChannelWire, BATCH_BYTES};
use crate::vfs::{BatchGet, BatchSink, VfsResult};
use futures_util::future::{join, join_all, FutureExt, LocalBoxFuture};
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::collections::BTreeMap;
use std::io;

/// Bytes handed to the sink per call: the engine's copy block.
const SINK_PORTION: usize = 256 * 1024;

/// The members of a packet that are on the wire.
type Running<'a> = FuturesUnordered<LocalBoxFuture<'a, (usize, Fetched)>>;

/// How a member ended before it reached the sink.
enum Fetched {
    /// The bytes read; `changed`: fewer than listed, or more behind them.
    Content { bytes: Vec<u8>, changed: bool },
    /// Nothing could be read.
    Failed(io::Error),
}

/// `get_batch` of the SFTP backend: `Unsupported` before anything is sent
/// when a packet cannot run on a pool channel.
pub(super) fn run(
    backend: &SftpBackend,
    items: &[BatchGet],
    sink: &mut dyn BatchSink,
) -> VfsResult<()> {
    if items.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Ein Paket braucht mindestens zwei Dateien",
        ));
    }
    let lease = channel_or_unsupported(backend.pool.lease(backend)?)?;
    let wire = ChannelWire::new(lease.channel().clone(), backend.connection.clone());
    let result = backend.rt.block_on(get_all(&wire, items, sink));
    drop(lease);
    result
}

/// Hands every item to `sink` in request order. A sink error stops the
/// packet: nothing new is started, the members on the wire finish and close
/// their handles, then the error is returned.
pub(super) async fn get_all(
    wire: &dyn BatchWire,
    items: &[BatchGet],
    sink: &mut dyn BatchSink,
) -> io::Result<()> {
    let window = window(wire, items.len());
    let mut running: Running<'_> = FuturesUnordered::new();
    let mut ready: BTreeMap<usize, Fetched> = BTreeMap::new();
    let mut started = 0;
    let mut delivered = 0;
    let mut sink_error = None;
    while delivered < items.len() {
        while sink_error.is_none() && started < items.len() && running.len() < window {
            if let Some(item) = items.get(started) {
                running.push(member(wire, started, item).boxed_local());
            }
            started += 1;
        }
        let Some((index, fetched)) = running.next().await else {
            break;
        };
        ready.insert(index, fetched);
        while sink_error.is_none() {
            let (Some(item), Some(fetched)) = (items.get(delivered), ready.remove(&delivered))
            else {
                break;
            };
            match deliver(sink, delivered, item, fetched) {
                Ok(()) => delivered += 1,
                Err(error) => sink_error = Some(error),
            }
        }
        if sink_error.is_some() {
            break;
        }
    }
    while running.next().await.is_some() {}
    match sink_error {
        Some(error) => Err(error),
        None if delivered < items.len() => Err(io::Error::other(
            "Paket-Download lieferte nicht alle Dateien",
        )),
        None => Ok(()),
    }
}

async fn member(wire: &dyn BatchWire, index: usize, item: &BatchGet) -> (usize, Fetched) {
    let handle = match wire.open_read(&item.path).await {
        Ok(handle) => handle,
        Err(failure) => return (index, Fetched::Failed(failure.error)),
    };
    let fetched = read_all(wire, &handle, item.size).await;
    // The handle counts against the server's limit until it is closed.
    let _ = wire.close(&handle).await;
    (index, fetched)
}

/// All READs of `[0, size)` plus the probe at `size` go out together. A
/// short answer in the middle (a server that cuts a READ) is read again
/// from where the bytes end.
async fn read_all(wire: &dyn BatchWire, handle: &str, size: u64) -> Fetched {
    if size > BATCH_BYTES {
        return Fetched::Failed(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Datei ist größer als ein Paket",
        ));
    }
    let chunk = u64::from(wire.read_chunk().max(1));
    let requests: Vec<(u64, u32)> = (0..size)
        .step_by(chunk as usize)
        .map(|offset| (offset, (size - offset).min(chunk) as u32))
        .collect();
    let reads = join_all(
        requests
            .iter()
            .map(|(offset, len)| wire.read(handle, *offset, *len)),
    );
    let (parts, probe) = join(reads, wire.read(handle, size, 1)).await;
    let mut bytes = Vec::with_capacity(size as usize);
    for ((offset, len), part) in requests.iter().zip(parts) {
        match part {
            Ok(part) if bytes.len() as u64 == *offset => {
                bytes.extend_from_slice(&part[..part.len().min(*len as usize)])
            }
            Ok(_) => {}
            Err(failure) => return Fetched::Failed(failure.error),
        }
    }
    while (bytes.len() as u64) < size {
        let offset = bytes.len() as u64;
        let len = (size - offset).min(chunk) as u32;
        match wire.read(handle, offset, len).await {
            Ok(part) if part.is_empty() => break,
            Ok(part) => bytes.extend_from_slice(&part[..part.len().min(len as usize)]),
            Err(failure) => return Fetched::Failed(failure.error),
        }
    }
    let grown = match probe {
        Ok(behind) => !behind.is_empty(),
        Err(failure) => return Fetched::Failed(failure.error),
    };
    let changed = (bytes.len() as u64) < size || grown;
    Fetched::Content { bytes, changed }
}

/// One finished member to the sink.
fn deliver(
    sink: &mut dyn BatchSink,
    index: usize,
    item: &BatchGet,
    fetched: Fetched,
) -> io::Result<()> {
    match fetched {
        Fetched::Failed(error) => sink.failed(index, error),
        Fetched::Content { bytes, changed } => {
            sink.begin(index, item.size)?;
            for portion in bytes.chunks(SINK_PORTION) {
                sink.data(index, portion)?;
            }
            let result = if changed {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Quelle wurde während der Übertragung geändert",
                ))
            } else {
                Ok(())
            };
            sink.end(index, result)
        }
    }
}
