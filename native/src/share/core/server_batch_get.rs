//! Host side of GetBatch: many small files in one stream, in request order.
//! Each item is announced with its length (or fails on its own), streamed,
//! and closed with a result that reports a source changed while it was read.
use std::io::{self, Read};

use iroh::endpoint::SendStream;
use tokio::sync::mpsc;

use crate::share::framing::{reply, reply_err, send_tagged, TAG_DATA};
use crate::share::fs::{ResolvedTarget, CHUNK};
use crate::share::fs_access::FsAccess;
use crate::share::server_transfer::{stat_item, STREAM_BUFFER_CHUNKS};
use crate::share::wire::{FsBatchGet, FsResponse};
use crate::vfs::VfsMeta;

use super::fs_dispatch::BatchAuthority;

enum Event {
    /// The item follows with exactly this many bytes.
    Begin(u64),
    Data(Vec<u8>),
    /// All bytes sent and the source unchanged.
    Done,
    /// The item could not be read, or failed or changed after `Begin`.
    Fail(io::Error),
}

/// The `slot` (one transfer admission for the whole batch) stays held until
/// the worker has read every item or the client went away.
pub(super) async fn serve<G: Send + 'static>(
    mut send: SendStream,
    items: Vec<FsBatchGet>,
    authority: BatchAuthority,
    slot: G,
) -> io::Result<()> {
    let (events_tx, mut events) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let worker = crate::share::blocking::spawn_holding("Share batch download", slot, move || {
        for item in &items {
            if !serve_item(item, &authority, &events_tx) {
                break;
            }
        }
        Ok(())
    });
    let sent = forward(&mut send, &mut events).await;
    // A vanished client stops the worker at its next item or chunk.
    drop(events);
    worker.join().await?;
    sent
}

async fn forward(send: &mut SendStream, events: &mut mpsc::Receiver<Event>) -> io::Result<()> {
    reply(send, FsResponse::Ready).await?;
    while let Some(event) = events.recv().await {
        match event {
            Event::Begin(size) => reply(send, FsResponse::Data { size }).await?,
            Event::Data(bytes) => send_tagged(send, TAG_DATA, &bytes).await?,
            Event::Done => reply(send, FsResponse::Ok).await?,
            Event::Fail(error) => reply_err(send, error).await?,
        }
    }
    Ok(())
}

/// Sends one item; false once the client is gone.
fn serve_item(item: &FsBatchGet, authority: &BatchAuthority, events: &mpsc::Sender<Event>) -> bool {
    let (target, before, mut reader) = match authority.admit(|access| open_item(access, item)) {
        Ok(opened) => opened,
        Err(error) => return events.blocking_send(Event::Fail(error)).is_ok(),
    };
    if events.blocking_send(Event::Begin(item.size)).is_err() {
        return false;
    }
    let end = match stream_bytes(&mut *reader, item.size, events) {
        Ok(false) => return false,
        Ok(true) => {
            drop(reader);
            match unchanged(&target, &before, item.id.as_deref()) {
                Ok(()) => Event::Done,
                Err(error) => Event::Fail(error),
            }
        }
        Err(error) => Event::Fail(error),
    };
    events.blocking_send(end).is_ok()
}

fn open_item(
    access: &FsAccess,
    item: &FsBatchGet,
) -> io::Result<(ResolvedTarget, VfsMeta, Box<dyn Read + Send>)> {
    let target = access.resolve(&item.path)?;
    let before = stat_item(&*target.backend, &target.path, item.id.as_deref())?;
    // Links are never followed; such an item fails on its own.
    if before.is_dir || before.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Im Paket werden nur reguläre Dateien gelesen",
        ));
    }
    match target.backend.read_size(&target.path, before.size)? {
        Some(size) if size == item.size => {}
        Some(size) => {
            return Err(changed(format!(
                "Quelle wurde seit der Auflistung geändert ({size} statt {} Bytes)",
                item.size
            )))
        }
        None => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Die Länge entsteht erst beim Lesen; die Datei wird einzeln übertragen",
            ))
        }
    }
    let reader = target
        .backend
        .open_read_id(&target.path, item.id.as_deref())?;
    Ok((target, before, reader))
}

/// Streams exactly `size` bytes; Ok(false) once the client is gone.
fn stream_bytes(
    reader: &mut dyn Read,
    size: u64,
    events: &mpsc::Sender<Event>,
) -> io::Result<bool> {
    let mut remaining = size;
    while remaining > 0 {
        let length = usize::try_from(remaining).map_or(CHUNK, |remaining| remaining.min(CHUNK));
        let mut buffer = vec![0u8; length];
        let read = match reader.read(&mut buffer) {
            Ok(0) => return Err(changed("Quelle wurde während des Lesens gekürzt".into())),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        buffer.truncate(read);
        remaining -= read as u64;
        if events.blocking_send(Event::Data(buffer)).is_err() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Size and modification time after the read equal those before it.
fn unchanged(target: &ResolvedTarget, before: &VfsMeta, id: Option<&str>) -> io::Result<()> {
    let after = stat_item(&*target.backend, &target.path, id)?;
    if after.size == before.size && after.mtime_ms == before.mtime_ms {
        Ok(())
    } else {
        Err(changed("Quelle wurde während des Lesens geändert".into()))
    }
}

fn changed(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
