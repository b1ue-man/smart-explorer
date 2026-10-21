//! Upload packets: many small new files on one pool channel. Every member
//! runs the stage chain of a single upload (an exclusively created private
//! stage, all WRITEs at once, CLOSE, RENAME without replacing, a numbered
//! name when the name is taken), and the chains of up to `open-handles`
//! members overlap, so a packet costs about four round trips instead of
//! four per file. Nothing is published before the stage is written and
//! closed; a result that is unknown is never labeled as "unpublished".
use super::backend::SftpBackend;
use super::batch_ops::{
    channel_or_unsupported, stage_name, window, BatchWire, ChannelWire, WireError, BATCH_BYTES,
};
use crate::transfer::engine_names::next_numbered;
use crate::vfs::{congestion_of, unpublished_error, BatchPut, BatchPutOutcome, VfsResult};
use futures_util::future::{join_all, FutureExt, LocalBoxFuture};
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::io::{self, Read};

/// The members of a packet that are on the wire.
type Running<'a> = FuturesUnordered<LocalBoxFuture<'a, (usize, BatchPutOutcome)>>;

/// Stage names tried per member: the name is random, so a second attempt
/// only happens when another actor took exactly that name.
const STAGE_ATTEMPTS: usize = 8;

/// `put_batch` of the SFTP backend: `Unsupported` before anything is read
/// when a packet cannot run on a pool channel (the engine then uploads the
/// files one by one).
pub(super) fn run(
    backend: &SftpBackend,
    entries: &[BatchPut],
    data: &mut dyn Read,
) -> VfsResult<Vec<BatchPutOutcome>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    if entries.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Ein Paket braucht mindestens zwei Dateien",
        ));
    }
    let lease = channel_or_unsupported(backend.pool.lease(backend)?)?;
    let wire = ChannelWire::new(lease.channel().clone(), backend.connection.clone());
    let outcomes = backend.rt.block_on(put_all(&wire, entries, data));
    drop(lease);
    Ok(outcomes)
}

/// One outcome per entry, in entry order. The bytes of an entry are read
/// when its member starts, so at most `window` entries are in memory.
pub(super) async fn put_all(
    wire: &dyn BatchWire,
    entries: &[BatchPut],
    data: &mut dyn Read,
) -> Vec<BatchPutOutcome> {
    let window = window(wire, entries.len());
    let mut running: Running<'_> = FuturesUnordered::new();
    let mut done: Vec<Option<BatchPutOutcome>> = entries.iter().map(|_| None).collect();
    for (index, entry) in entries.iter().enumerate() {
        while running.len() >= window {
            collect(&mut running, &mut done).await;
        }
        match read_member(data, entry.size) {
            Ok(bytes) => {
                running.push(member(wire, index, &entry.path, bytes).boxed_local());
                // Its first request goes out now, not after the packet is read.
                if let Some(Some((first, outcome))) = running.next().now_or_never() {
                    done[first] = Some(outcome);
                }
            }
            Err(error) => {
                // Not even this entry could be read: it and every later one
                // never started, the running members finish.
                done[index] = Some(BatchPutOutcome::Failed(error));
                break;
            }
        }
    }
    while collect(&mut running, &mut done).await {}
    done.into_iter()
        .map(|outcome| {
            outcome.unwrap_or_else(|| {
                BatchPutOutcome::Failed(unpublished_error(
                    io::ErrorKind::Interrupted,
                    "Paket abgebrochen",
                ))
            })
        })
        .collect()
}

async fn collect(running: &mut Running<'_>, done: &mut [Option<BatchPutOutcome>]) -> bool {
    match running.next().await {
        Some((index, outcome)) => {
            if let Some(slot) = done.get_mut(index) {
                *slot = Some(outcome);
            }
            true
        }
        None => false,
    }
}

/// The next `size` bytes of `data`. An error is never retried (`read_exact`
/// would repeat `Interrupted`, which is how a canceled packet stream ends).
fn read_member(data: &mut dyn Read, size: u64) -> io::Result<Vec<u8>> {
    if size > BATCH_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Datei ist größer als ein Paket",
        ));
    }
    let mut bytes = vec![0u8; size as usize];
    let mut filled = 0;
    while filled < bytes.len() {
        match data.read(&mut bytes[filled..])? {
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Paketdaten enden vor dem Ende der Datei",
                ))
            }
            read => filled += read,
        }
    }
    Ok(bytes)
}

async fn member(
    wire: &dyn BatchWire,
    index: usize,
    destination: &str,
    bytes: Vec<u8>,
) -> (usize, BatchPutOutcome) {
    (index, put_member(wire, destination, bytes).await)
}

/// One file: stage, write, close, publish.
async fn put_member(wire: &dyn BatchWire, destination: &str, bytes: Vec<u8>) -> BatchPutOutcome {
    let (stage, handle) = match open_stage(wire, destination).await {
        Ok(opened) => opened,
        Err(error) => return BatchPutOutcome::Failed(error),
    };
    if let Err(failure) = write_all(wire, &handle, &bytes).await {
        let _ = wire.close(&handle).await;
        let _ = wire.remove(&stage).await;
        return BatchPutOutcome::Failed(judged(failure, &stage));
    }
    if let Err(failure) = wire.close(&handle).await {
        let _ = wire.remove(&stage).await;
        return BatchPutOutcome::Failed(judged(failure, &stage));
    }
    publish(wire, &stage, destination).await
}

/// Exclusively creates a private stage next to `destination`; returns its
/// name and handle.
async fn open_stage(wire: &dyn BatchWire, destination: &str) -> io::Result<(String, String)> {
    for _ in 0..STAGE_ATTEMPTS {
        let stage = stage_name(destination);
        match wire.open_new(&stage).await {
            Ok(handle) => return Ok((stage, handle)),
            Err(failure) => {
                if !taken(wire, &failure, &stage).await {
                    return Err(judged(failure, &stage));
                }
            }
        }
    }
    Err(unpublished_error(
        io::ErrorKind::AlreadyExists,
        format!("Keine freie Upload-Stufe für „{destination}“"),
    ))
}

/// Whether the refusal of an exclusive OPEN says the name is taken. SFTP v3
/// has no such status, a server answers a plain failure; the name is then
/// looked at.
async fn taken(wire: &dyn BatchWire, failure: &WireError, stage: &str) -> bool {
    if !failure.answered || congestion_of(&failure.error).is_some() {
        return false;
    }
    match failure.error.kind() {
        io::ErrorKind::AlreadyExists => true,
        io::ErrorKind::Other => wire.exists(stage).await.unwrap_or(false),
        _ => false,
    }
}

/// All WRITEs of the file are on the wire before the first answer; the first
/// failure in file order is the result.
async fn write_all(wire: &dyn BatchWire, handle: &str, bytes: &[u8]) -> Result<(), WireError> {
    let chunk = wire.write_chunk(handle.len()).max(1);
    let writes = bytes
        .chunks(chunk)
        .enumerate()
        .map(|(number, part)| wire.write(handle, (number * chunk) as u64, part.to_vec()));
    for result in join_all(writes).await {
        result?;
    }
    Ok(())
}

/// Renames the written stage to `destination` or, when that name is taken
/// (by anything), to the next free "name (n)". A failure without a status
/// reply has an unknown result: no further attempt.
async fn publish(wire: &dyn BatchWire, stage: &str, destination: &str) -> BatchPutOutcome {
    let (parent, name) = match destination.rfind('/') {
        Some(index) => destination.split_at(index + 1),
        None => ("", destination),
    };
    let numbered = next_numbered(name).map(|numbered| format!("{parent}{numbered}"));
    for candidate in std::iter::once(destination.to_string()).chain(numbered) {
        let failure = match wire.rename(stage, &candidate).await {
            Ok(()) => return BatchPutOutcome::Published(candidate),
            Err(failure) => failure,
        };
        if !failure.answered {
            return BatchPutOutcome::Failed(judged(failure, stage));
        }
        if wire.exists(&candidate).await.unwrap_or(false) {
            continue;
        }
        let _ = wire.remove(stage).await;
        return BatchPutOutcome::Failed(judged(failure, stage));
    }
    let _ = wire.remove(stage).await;
    BatchPutOutcome::Failed(unpublished_error(
        io::ErrorKind::AlreadyExists,
        format!("Kein freier Name für „{destination}“"),
    ))
}

/// The error of a failed step. A refusal as congestion stays one (the file
/// waits for the server). A status reply before the rename changed nothing:
/// "unpublished". Without a reply the stage may exist and the result is
/// unknown; the text names the stage.
fn judged(failure: WireError, stage: &str) -> io::Error {
    let WireError { error, answered } = failure;
    if congestion_of(&error).is_some() {
        return error;
    }
    if answered {
        return unpublished_error(error.kind(), error.to_string());
    }
    io::Error::new(
        error.kind(),
        format!("{error}; mögliche Upload-Stufe „{stage}“ bleibt erhalten"),
    )
}
