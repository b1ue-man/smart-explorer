//! The read side of the host's transfers: `Read`, `ReadAt` (resume or one
//! range, `ranges_v1`) and the optional LZ4 packing of the content
//! (`lz4_v1`). A file is read on a blocking worker one chunk ahead of the
//! stream, so one disk read overlaps one network send.
use std::io::{self, Read};
use std::time::{Duration, Instant};

use iroh::endpoint::SendStream;
use tokio::sync::{mpsc, oneshot};

use crate::share::blocking;
use crate::share::core::eio;
use crate::share::framing::lz4_stream::{self, Lz4Stream};
use crate::share::framing::{reply, reply_err, send_tagged, TAG_DATA, TAG_DATA_LZ4};
use crate::share::fs;
use crate::share::fs_access::FsAccess;
use crate::share::io_deadline;
use crate::share::transfer_meter as stats;
use crate::share::wire::{FsFileEvidence, FsResponse};
use crate::vfs::{Backend, VfsMeta};

use super::STREAM_BUFFER_CHUNKS;

/// What a Read or ReadAt names: the whole file, a provider ID among equal
/// names, the rest from `offset` (resume) or `len` bytes from `offset`.
pub(in crate::share) struct ReadSource {
    pub(in crate::share) path: String,
    pub(in crate::share) id: Option<String>,
    pub(in crate::share) offset: u64,
    /// 0 = to the end of the file.
    pub(in crate::share) len: u64,
    /// The client accepts LZ4 blocks.
    pub(in crate::share) lz4: bool,
    /// `evidence_v1` ReadAt: the header is `EvidencedData`.
    pub(in crate::share) evidence: bool,
}

/// One chunk on its way to the stream, already in its frame form.
struct ReadChunk {
    tag: u8,
    bytes: Vec<u8>,
    /// Raw bytes of the chunk (what the stream's window and the client's
    /// accounting count).
    raw: usize,
}

/// Metadata of `path`, or of the item `id` among equal names in its folder
/// (ID providers behind an export); without an ID exactly `stat`.
pub(in crate::share) fn stat_item(
    backend: &dyn Backend,
    path: &str,
    id: Option<&str>,
) -> io::Result<VfsMeta> {
    let Some(id) = id else {
        return backend.stat(path);
    };
    let (parent, name) = path
        .rsplit_once('/')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Pfad ohne Ordner"))?;
    let parent = if parent.is_empty() { "/" } else { parent };
    backend
        .list_dir(parent)?
        .into_iter()
        .find(|entry| entry.name == name && entry.id.as_deref() == Some(id))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Datei mit der Kennung {id} fehlt: {path}"),
            )
        })
}

/// The `slot` (transfer admission) stays held until the worker ends; a
/// client that reads nothing for `stall` loses it.
pub(in crate::share) async fn read_file<G: Send + 'static>(
    mut send: SendStream,
    source: ReadSource,
    access: FsAccess,
    slot: G,
    stall: Duration,
) -> io::Result<()> {
    let (ready_tx, ready_rx) = oneshot::channel();
    let (data_tx, mut data_rx) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let live = access.clone();
    let lz4 = lz4_stream::negotiated(source.lz4).then(Lz4Stream::new);
    let packing = lz4.clone();
    let worker = blocking::spawn_holding("Share read", slot, move || {
        read_worker(source, access, ready_tx, data_tx, packing)
    });

    let (size, evidence) = match ready_rx.await {
        Ok(Ok(opened)) => opened,
        Ok(Err(error)) => return reply_err(&mut send, error).await,
        Err(_) => return Err(worker_exit(worker.join().await)),
    };
    live.check_read()?;
    stats::stream_started();
    let header = match evidence {
        Some(evidence) => FsResponse::EvidencedData { size, evidence },
        None => FsResponse::Data { size },
    };
    let header = reply(&mut send, header);
    io_deadline::run_for("Share read data", stall, header).await?;
    loop {
        // Waiting here is waiting for the reader worker (the disk).
        let waiting = stats::Timer::start();
        let next = data_rx.recv().await;
        waiting.stop(stats::Phase::DiskWait);
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk?;
        live.check_read()?;
        let started = Instant::now();
        let sent = send_tagged(&mut send, chunk.tag, &chunk.bytes);
        // Returning drops the channel: the worker stops and frees the slot.
        io_deadline::run_for("Share read data", stall, sent).await?;
        let blocked = started.elapsed();
        if stats::enabled() {
            stats::meter().add(stats::Phase::NetWait, blocked);
            stats::bytes(chunk.raw, chunk.bytes.len());
        }
        if let Some(lz4) = &lz4 {
            lz4.record_send(chunk.raw, blocked, Instant::now());
        }
    }
    live.check_read()?;
    worker.join().await
}

fn worker_exit(joined: io::Result<()>) -> io::Error {
    match joined {
        Ok(()) => eio("Share read worker exited without a result"),
        Err(error) => error,
    }
}

fn read_worker(
    source: ReadSource,
    access: FsAccess,
    ready: oneshot::Sender<io::Result<(u64, Option<FsFileEvidence>)>>,
    chunks: mpsc::Sender<io::Result<ReadChunk>>,
    lz4: Option<Lz4Stream>,
) -> io::Result<()> {
    let (size, evidence, mut reader) = match open_source(&access, &source) {
        Ok(opened) => opened,
        Err(error) => {
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };
    if ready.send(Ok((size, evidence))).is_err() {
        return Ok(());
    }
    loop {
        let mut buffer = vec![0u8; fs::CHUNK];
        let reading = stats::Timer::start();
        let read = reader.read(&mut buffer);
        reading.stop(stats::Phase::DiskRead);
        let read = match read {
            Ok(read) => read,
            Err(error) => {
                let _ = chunks.blocking_send(Err(error));
                return Ok(());
            }
        };
        if read == 0 {
            return Ok(());
        }
        buffer.truncate(read);
        let chunk = match lz4.as_ref().and_then(|stream| stream.pack(&buffer)) {
            Some(block) => ReadChunk {
                tag: TAG_DATA_LZ4,
                bytes: block,
                raw: read,
            },
            None => ReadChunk {
                tag: TAG_DATA,
                bytes: buffer,
                raw: read,
            },
        };
        if chunks.blocking_send(Ok(chunk)).is_err() {
            return Ok(());
        }
    }
}

/// Bytes a Read or ReadAt announces and the reader that delivers them. The
/// arithmetic is checked: `offset` behind the end is `InvalidInput`, and the
/// announced length is `min(size - offset, len)` (`len` 0 = `size - offset`);
/// the reader is cut after exactly that many bytes. A file that shrinks
/// while it is read ends the stream early, which the client reports as a
/// source that changed. An `evidence` read also reports what the host read
/// about the file before it opened it.
fn open_source(
    access: &FsAccess,
    source: &ReadSource,
) -> io::Result<(u64, Option<FsFileEvidence>, Box<dyn Read + Send>)> {
    let target = access.resolve(&source.path)?;
    let metadata = stat_item(&*target.backend, &target.path, source.id.as_deref())?;
    if metadata.is_dir {
        return Err(eio("Ordner kann nicht als Datei gelesen werden"));
    }
    let evidence = if source.evidence {
        let mut scopes = crate::share::host_evidence::Scopes::default();
        crate::share::host_evidence::of_meta(access, &source.path, &target, &metadata, &mut scopes)
    } else {
        None
    };
    if source.offset == 0 && source.len == 0 {
        let reader = match source.id.as_deref() {
            None => target.backend.open_read(&target.path)?,
            Some(id) => target.backend.open_read_id(&target.path, Some(id))?,
        };
        return Ok((metadata.size, evidence, reader));
    }
    let rest = metadata.size.checked_sub(source.offset).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Fortsetzungspunkt liegt hinter dem Dateiende",
        )
    })?;
    let remaining = if source.len > 0 {
        rest.min(source.len)
    } else {
        rest
    };
    let reader = if source.offset == 0 {
        match source.id.as_deref() {
            None => target.backend.open_read(&target.path)?,
            Some(id) => target.backend.open_read_id(&target.path, Some(id))?,
        }
    } else {
        target
            .backend
            .open_read_at(&target.path, source.id.as_deref(), source.offset)?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Diese Freigabe kann nicht mitten in der Datei weiterlesen",
                )
            })?
    };
    Ok((remaining, evidence, Box::new(reader.take(remaining))))
}
