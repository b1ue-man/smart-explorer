//! Packets of small files over SMB with compound requests (plan U46-3).
//!
//! A new file takes two round trips instead of five: one compound
//! `CREATE(FileCreate, stage) + WRITE [+ CLOSE]`, then the existing
//! no-replace rename (`replace.rs`). A file is read with one compound
//! `CREATE(FileOpen) + READ [+ CLOSE]`. Larger files send their remaining
//! WRITE/READ requests at once after the first compound. Every member runs
//! at the same time; the credit window of the connection paces them
//! (`Connection::execute_compound` waits for the credits of a whole chain).
//!
//! The logic is generic over [`SmbWire`] so it runs against a scripted wire
//! in the tests. The real wire (`batch_wire.rs`) builds the compounds from
//! the pieces of `wire.rs` and puts FILE_OPEN_REPARSE_POINT on every CREATE:
//! smb2's `read_file_compound*`/`write_file_compound*` leave the attributes
//! out, so a reparse point would be followed.
pub(super) use super::batch_wire::{get, put};
use crate::transfer::engine_names::next_numbered;
use crate::vfs::{
    congestion_of, fit_stage_name, unpublished_error, BatchGet, BatchLimits, BatchPut,
    BatchPutOutcome, BatchSink,
};
use futures_util::future::join_all;
use futures_util::stream::{FuturesUnordered, StreamExt};
use smb2::types::FileId;
use std::future::Future;
use std::io::{self, Read};
use std::pin::Pin;
use std::task::Poll;

/// Files per packet (the cap of every packet path, `share/core/batch_wire.rs`).
pub(super) const BATCH_MAX_FILES: usize = 256;
/// Bytes per packet: the memory the engine reserves and the catch-up cost.
pub(super) const BATCH_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// A 64-bit random stage name collides only with a foreign file of that name.
const STAGE_ATTEMPTS: usize = 8;
/// Largest chunk handed to the sink at once.
const SINK_CHUNK: usize = 256 * 1024;
const CHANGED: &str = "Quelle wurde während der Übertragung geändert";

/// A failed request. `answered`: the server replied with a status, so the
/// request had no effect; otherwise (transport, timeout) the result is
/// unknown. `created`: the CREATE of this call succeeded, so the stage exists
/// and is ours (a refused CREATE leaves a foreign file of that name alone).
pub(super) struct WireError {
    pub(super) error: io::Error,
    pub(super) answered: bool,
    pub(super) created: bool,
}

/// Not `Send`: polled on the thread of the calling `block_on`.
pub(super) type WireFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, WireError>> + 'a>>;

/// An open handle of this client.
pub(super) struct FileState {
    pub(super) id: FileId,
    /// The compound already closed it.
    pub(super) closed: bool,
}

/// The first READ of a file.
pub(super) struct ReadResult {
    /// EndOfFile of the CREATE answer: the size at the moment of the open.
    pub(super) size: u64,
    pub(super) data: Vec<u8>,
    /// Still open when the file needs more READs.
    pub(super) file: Option<FileState>,
    /// The entry carries the reparse attribute and was opened as itself.
    pub(super) reparse: bool,
}

pub(super) trait SmbWire {
    /// CREATE(FileCreate) of `stage` + WRITE of `data[..first_block]`, plus
    /// CLOSE when that is all of `data`; no FLUSH.
    fn create_write_close_new<'a>(
        &'a self,
        stage: &'a str,
        data: &'a [u8],
        first_block: usize,
    ) -> WireFuture<'a, FileState>;
    fn write_more<'a>(
        &'a self,
        file: &'a FileState,
        offset: u64,
        data: &'a [u8],
    ) -> WireFuture<'a, ()>;
    fn close<'a>(&'a self, file: &'a FileState) -> WireFuture<'a, ()>;
    /// The compound `CREATE(DELETE) + SET_INFO(rename, no replace) + CLOSE`.
    fn rename_no_replace<'a>(&'a self, from: &'a str, to: &'a str) -> WireFuture<'a, ()>;
    fn remove<'a>(&'a self, path: &'a str) -> WireFuture<'a, ()>;
    /// CREATE(FileOpen) + READ of the first `min(expected, max_read)` bytes,
    /// plus CLOSE when that is the whole expected file.
    fn open_read_whole<'a>(&'a self, path: &'a str, expected: u64) -> WireFuture<'a, ReadResult>;
    fn read_more<'a>(
        &'a self,
        file: &'a FileState,
        offset: u64,
        len: usize,
    ) -> WireFuture<'a, Vec<u8>>;
    /// A data reparse point (deduplicated, cloud placeholder) read the way
    /// the single path reads it; `None`: it is a redirecting link.
    fn read_followed<'a>(&'a self, path: &'a str) -> WireFuture<'a, Option<Vec<u8>>>;
    fn max_write(&self) -> usize;
    fn max_read(&self) -> usize;
}

pub(super) fn limits(live: bool) -> Option<BatchLimits> {
    live.then_some(BatchLimits {
        max_files: BATCH_MAX_FILES,
        max_bytes: BATCH_MAX_BYTES,
    })
}

fn random_hex() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::getrandom(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or_default();
        bytes = (nanos ^ u64::from(std::process::id()).rotate_left(32)).to_le_bytes();
    }
    format!("{:016x}", u64::from_le_bytes(bytes))
}

fn split_name(path: &str) -> (&str, &str) {
    path.rfind('/')
        .map_or(("", path), |index| path.split_at(index + 1))
}

/// `<destination>.se-upload-<16 hex>` next to the destination.
pub(super) fn stage_name(destination: &str) -> String {
    let (parent, name) = split_name(destination);
    let tail = format!(".se-upload-{}", random_hex());
    format!("{parent}{}", fit_stage_name("", name, &tail))
}

fn unsupported(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message.to_string())
}

/// A failure that certainly published nothing. Overload stays congestion so
/// the flow backs off.
fn unpublished(error: io::Error) -> io::Error {
    if congestion_of(&error).is_some() {
        return error;
    }
    let (kind, text) = (error.kind(), error.to_string());
    unpublished_error(kind, text)
}

fn with_stage(error: io::Error, stage: &str) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("{error}; Stufe „{stage}“ ist möglicherweise angelegt"),
    )
}

/// The member failed: an answered refusal removes the stage that is ours; an
/// unanswered request has an unknown result and is never repeated.
async fn refuse<W: SmbWire + ?Sized>(wire: &W, stage: &str, error: WireError) -> BatchPutOutcome {
    if !error.answered {
        return BatchPutOutcome::Failed(with_stage(error.error, stage));
    }
    if error.created {
        let _ = wire.remove(stage).await;
    }
    BatchPutOutcome::Failed(unpublished(error.error))
}

async fn put_member<W: SmbWire + ?Sized>(
    wire: &W,
    destination: &str,
    bytes: Vec<u8>,
) -> BatchPutOutcome {
    let max_write = wire.max_write().max(1);
    let block = bytes.len().min(max_write);
    let mut opened = None;
    for _ in 0..STAGE_ATTEMPTS {
        let stage = stage_name(destination);
        match wire.create_write_close_new(&stage, &bytes, block).await {
            Ok(file) => {
                opened = Some((stage, file));
                break;
            }
            Err(error)
                if error.answered
                    && !error.created
                    && error.error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return refuse(wire, &stage, error).await,
        }
    }
    let Some((stage, file)) = opened else {
        let error = unpublished_error(io::ErrorKind::AlreadyExists, "Keine freie Upload-Stufe");
        return BatchPutOutcome::Failed(error);
    };
    if !file.closed {
        let mut writes = Vec::new();
        let mut offset = block;
        while offset < bytes.len() {
            let end = (offset + max_write).min(bytes.len());
            writes.push(wire.write_more(&file, offset as u64, &bytes[offset..end]));
            offset = end;
        }
        let failed = join_all(writes).await.into_iter().find_map(Result::err);
        let closed = match failed {
            None => wire.close(&file).await.err(),
            Some(error) => {
                if error.answered {
                    let _ = wire.close(&file).await;
                }
                Some(error)
            }
        };
        if let Some(mut error) = closed {
            error.created = true;
            return refuse(wire, &stage, error).await;
        }
    }
    let (parent, name) = split_name(destination);
    for candidate in std::iter::once(name.to_string()).chain(next_numbered(name)) {
        let candidate = format!("{parent}{candidate}");
        match wire.rename_no_replace(&stage, &candidate).await {
            Ok(()) => return BatchPutOutcome::Published(candidate),
            Err(error) if error.answered && error.error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(mut error) => {
                error.created = true;
                return refuse(wire, &stage, error).await;
            }
        }
    }
    let _ = wire.remove(&stage).await;
    let message = format!("Kein freier Name für {destination}");
    BatchPutOutcome::Failed(unpublished_error(io::ErrorKind::AlreadyExists, message))
}

/// Fills `bytes` from `data`. `read_exact` would retry `Interrupted`, which
/// is how a cancelled packet stream reports itself.
fn read_entry(data: &mut dyn Read, bytes: &mut [u8]) -> io::Result<()> {
    let mut filled = 0;
    while filled < bytes.len() {
        match data.read(&mut bytes[filled..])? {
            0 => return Err(io::ErrorKind::UnexpectedEof.into()),
            read => filled += read,
        }
    }
    Ok(())
}

/// Polls the running members once; every finished one is stored.
async fn collect<T, F: Future<Output = (usize, T)>>(
    running: &mut FuturesUnordered<F>,
    results: &mut [Option<T>],
) {
    loop {
        let polled = std::future::poll_fn(|cx| match running.poll_next_unpin(cx) {
            Poll::Ready(Some(done)) => Poll::Ready(Some(done)),
            _ => Poll::Ready(None),
        })
        .await;
        match polled {
            Some((index, result)) => results[index] = Some(result),
            None => return,
        }
    }
}

/// Every entry as a new file. The members start in order and run together;
/// a failed read of the packet stream ends the starting: that entry fails
/// with the read error, the later ones certainly published nothing.
pub(super) async fn put_all<W: SmbWire + ?Sized>(
    wire: &W,
    entries: &[BatchPut],
    data: &mut dyn Read,
) -> Vec<BatchPutOutcome> {
    let mut results: Vec<Option<BatchPutOutcome>> = entries.iter().map(|_| None).collect();
    let mut running = FuturesUnordered::new();
    let mut stopped_at = None;
    for (index, entry) in entries.iter().enumerate() {
        let read = usize::try_from(entry.size)
            .map_err(|_| unsupported("Datei ist für ein Paket zu groß"))
            .and_then(|size| {
                let mut bytes = vec![0u8; size];
                read_entry(data, &mut bytes).map(|()| bytes)
            });
        match read {
            Ok(bytes) => {
                let destination = entry.path.as_str();
                running.push(async move { (index, put_member(wire, destination, bytes).await) });
                collect(&mut running, &mut results).await;
            }
            Err(error) => {
                results[index] = Some(BatchPutOutcome::Failed(error));
                stopped_at = Some(index);
                break;
            }
        }
    }
    while let Some((index, result)) = running.next().await {
        results[index] = Some(result);
    }
    let first_unstarted = stopped_at.map_or(entries.len(), |at| at + 1);
    results
        .into_iter()
        .enumerate()
        .map(|(index, result)| match result {
            Some(result) => result,
            None if index >= first_unstarted => BatchPutOutcome::Failed(unpublished_error(
                io::ErrorKind::Interrupted,
                "Paket abgebrochen",
            )),
            None => BatchPutOutcome::Failed(io::Error::other("Paket-Upload ohne Ergebnis")),
        })
        .collect()
}

/// What one member of a packet download ended with.
enum Fetched {
    Data(Vec<u8>),
    Changed,
    Failed(io::Error),
}

async fn fetch<W: SmbWire + ?Sized>(wire: &W, item: &BatchGet) -> Fetched {
    let first = match wire.open_read_whole(&item.path, item.size).await {
        Ok(first) => first,
        Err(error) => return Fetched::Failed(error.error),
    };
    let ReadResult {
        size,
        mut data,
        file,
        reparse,
    } = first;
    let same_size = size == item.size;
    let mut failure = None;
    if let Some(file) = file {
        // The rest of a file larger than one READ: all requests at once.
        if same_size && !reparse {
            let max_read = wire.max_read().max(1) as u64;
            let mut reads = Vec::new();
            let mut offset = data.len() as u64;
            while offset < item.size {
                let len = max_read.min(item.size - offset);
                reads.push(wire.read_more(&file, offset, len as usize));
                offset += len;
            }
            for part in join_all(reads).await {
                match part {
                    Ok(bytes) => data.extend_from_slice(&bytes),
                    Err(error) => failure = failure.or(Some(error.error)),
                }
            }
        }
        let _ = wire.close(&file).await;
    }
    if let Some(error) = failure {
        return Fetched::Failed(error);
    }
    if reparse && same_size {
        return match wire.read_followed(&item.path).await {
            Ok(Some(bytes)) if bytes.len() as u64 == item.size => Fetched::Data(bytes),
            Ok(Some(_)) => Fetched::Changed,
            Ok(None) => Fetched::Failed(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} ist eine Verknüpfung und wird nicht gelesen", item.path),
            )),
            Err(error) => Fetched::Failed(error.error),
        };
    }
    if !same_size || data.len() as u64 != item.size {
        return Fetched::Changed;
    }
    Fetched::Data(data)
}

fn deliver(
    sink: &mut dyn BatchSink,
    index: usize,
    item: &BatchGet,
    done: Fetched,
) -> io::Result<()> {
    match done {
        Fetched::Failed(error) => sink.failed(index, error),
        Fetched::Changed => {
            sink.begin(index, item.size)?;
            sink.end(
                index,
                Err(io::Error::new(io::ErrorKind::InvalidData, CHANGED)),
            )
        }
        Fetched::Data(bytes) => {
            sink.begin(index, item.size)?;
            for chunk in bytes.chunks(SINK_CHUNK) {
                sink.data(index, chunk)?;
            }
            sink.end(index, Ok(()))
        }
    }
}

/// Every item to `sink` in request order, whatever order the answers come
/// in. A sink error stops the delivery; members already started finish (and
/// close their handles) before the error is returned.
pub(super) async fn get_all<W: SmbWire + ?Sized>(
    wire: &W,
    items: &[BatchGet],
    sink: &mut dyn BatchSink,
) -> io::Result<()> {
    let mut arrived: Vec<Option<Fetched>> = items.iter().map(|_| None).collect();
    let mut running = FuturesUnordered::new();
    for (index, item) in items.iter().enumerate() {
        running.push(async move { (index, fetch(wire, item).await) });
        collect(&mut running, &mut arrived).await;
    }
    let mut next = 0;
    let mut failure = None;
    loop {
        while next < items.len() && arrived[next].is_some() {
            let done = arrived[next].take();
            if let (Some(done), true) = (done, failure.is_none()) {
                failure = deliver(sink, next, &items[next], done).err();
            }
            next += 1;
        }
        match running.next().await {
            Some((index, done)) => arrived[index] = Some(done),
            None => return failure.map_or(Ok(()), Err),
        }
    }
}
