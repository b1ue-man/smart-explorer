//! A resumable upload streamed chunk by chunk from memory: each full chunk
//! goes out as soon as the next bytes arrive, its memory reserved (plan K7),
//! and fast chunks double up to a bound.
//!
//! Two buffers (G3-08c, 2026-10-28): when memory for a second chunk can be
//! reserved, a full chunk is sent on a thread of its own while the writer
//! fills the next one, so reading the source and sending overlap instead of
//! taking turns. Chunks still go out one after the other (the protocol
//! needs each confirmed before the next starts at its offset): a chunk
//! leaves only after the one before it landed. Without that memory the
//! upload sends each chunk in turn, as before. The upload session URI and
//! its rules (never replay blindly, a 409 means an earlier attempt already
//! created the ID) are unchanged.
use super::new_object::NewObject;
use super::overload::http_status;
use super::resumable::{self, Bearer, ChunkSource, Resumable, Sent};
use super::GDriveBackend;
use crate::transfer::MemoryReservation;
use std::io::{self, Read};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// First chunk of a streamed upload: the 8 MiB of the spooled path, a
/// multiple of the protocol's 256 KiB granule.
pub(super) const FIRST_CHUNK: usize = resumable::CHUNK_SIZE;
/// Largest chunk (doubling keeps the granule multiple): at 1 Gbit/s 64 MiB
/// take about half a second, long against one request's fixed cost; larger
/// chunks would only hold more memory per upload.
const MAX_CHUNK: usize = 64 * 1024 * 1024;
/// A chunk that went out faster than this doubles the next one, so the fixed
/// cost of each chunk request (a round trip plus the server's commit) stays
/// small against the time its bytes take.
const CHUNK_TARGET: Duration = Duration::from_secs(1);

/// The upload state a chunk on its way carries and brings back.
struct Flight {
    upload: Option<Resumable>,
    settled: bool,
    buffer: Vec<u8>,
    /// Upload offset where the chunk ends.
    limit: u64,
    result: io::Result<Duration>,
}

/// The in-memory side of one streamed upload.
pub(super) struct Stream {
    buffer: Vec<u8>,
    /// Current chunk length.
    capacity: usize,
    /// Upload offset of `buffer[0]`.
    start: u64,
    upload: Option<Resumable>,
    /// The reserved ID already exists (an earlier attempt committed it): the
    /// bytes only feed the MD5 that flush compares with it.
    settled: bool,
    /// Memory of every buffer (two of `capacity` once `overlap`).
    reservations: Vec<MemoryReservation>,
    /// `Some(true)`: a second buffer is reserved; `None`: not decided yet.
    overlap: Option<bool>,
    /// The empty second buffer while no chunk is on its way.
    spare: Option<Vec<u8>>,
    inflight: Option<JoinHandle<Flight>>,
}

impl Stream {
    /// A stream whose first chunk (`chunk` bytes) is already reserved.
    pub(super) fn new(chunk: usize, reservation: MemoryReservation) -> Self {
        Self {
            buffer: Vec::with_capacity(chunk),
            capacity: chunk,
            start: 0,
            upload: None,
            settled: false,
            reservations: vec![reservation],
            overlap: None,
            spare: None,
            inflight: None,
        }
    }

    pub(super) fn push(
        &mut self,
        backend: &GDriveBackend,
        object: &NewObject,
        total: u64,
        mut data: &[u8],
    ) -> io::Result<()> {
        while !data.is_empty() {
            let take = (self.capacity - self.buffer.len()).min(data.len());
            if take == 0 {
                // Only the final chunk stays buffered, and `write` refuses
                // bytes beyond the announced size.
                return Err(io::Error::other("Drive-Upload-Puffer ist voll"));
            }
            self.buffer.extend_from_slice(&data[..take]);
            data = &data[take..];
            let end = self.start + self.buffer.len() as u64;
            // A full chunk goes out once more bytes follow; the last one
            // waits for flush, which completes the upload.
            if self.buffer.len() == self.capacity && end < total {
                self.ship(backend, object, total, end)?;
            }
        }
        Ok(())
    }

    /// A full chunk leaves: on its own thread while the next one fills when
    /// a second buffer is reserved, else right here.
    fn ship(
        &mut self,
        backend: &GDriveBackend,
        object: &NewObject,
        total: u64,
        limit: u64,
    ) -> io::Result<()> {
        self.land(total)?;
        if !self.overlapping() {
            return self.send(backend, object, total, limit);
        }
        let mut next = self.spare.take().unwrap_or_default();
        next.reserve_exact(self.capacity);
        let data = std::mem::replace(&mut self.buffer, next);
        let start = std::mem::replace(&mut self.start, limit);
        let (backend, object) = (backend.clone(), object.clone());
        let (mut upload, mut settled) = (self.upload.take(), self.settled);
        let spawned = std::thread::Builder::new()
            .name("drive-chunk".into())
            .spawn(move || {
                let started = Instant::now();
                let sent = send_chunk(
                    &backend,
                    &object,
                    (&mut upload, &mut settled),
                    (start, &data),
                    total,
                    limit,
                );
                Flight {
                    upload,
                    settled,
                    buffer: data,
                    limit,
                    result: sent.map(|()| started.elapsed()),
                }
            });
        match spawned {
            Ok(flight) => {
                self.inflight = Some(flight);
                Ok(())
            }
            // The chunk went with the closure: this attempt fails and the
            // engine retries the file.
            Err(error) => Err(io::Error::other(format!(
                "Drive-Upload-Faden ließ sich nicht starten: {error}"
            ))),
        }
    }

    /// Whether a second buffer is reserved; decided at the first full chunk.
    fn overlapping(&mut self) -> bool {
        if self.overlap.is_none() {
            let reserved = crate::transfer::try_reserve_memory(self.capacity as u64);
            self.overlap = Some(reserved.is_some());
            if let Some(reservation) = reserved {
                self.reservations.push(reservation);
                self.spare = Some(Vec::with_capacity(self.capacity));
            }
        }
        self.overlap == Some(true)
    }

    /// Waits for the chunk on its way and takes its upload state back.
    fn land(&mut self, total: u64) -> io::Result<()> {
        let Some(flight) = self.inflight.take() else {
            return Ok(());
        };
        let flight = flight
            .join()
            .map_err(|_| io::Error::other("Drive-Upload-Faden brach ab"))?;
        self.upload = flight.upload;
        self.settled = flight.settled;
        let took = flight.result?;
        let mut buffer = flight.buffer;
        buffer.clear();
        self.spare = Some(buffer);
        if !self.settled {
            self.grow(took, total.saturating_sub(flight.limit));
        }
        Ok(())
    }

    /// Send the buffered bytes up to `limit` here (`total` completes the
    /// upload); a chunk still on its way lands first.
    pub(super) fn send(
        &mut self,
        backend: &GDriveBackend,
        object: &NewObject,
        total: u64,
        limit: u64,
    ) -> io::Result<()> {
        self.land(total)?;
        let started = Instant::now();
        send_chunk(
            backend,
            object,
            (&mut self.upload, &mut self.settled),
            (self.start, &self.buffer),
            total,
            limit,
        )?;
        if limit == total {
            return Ok(());
        }
        let took = started.elapsed();
        self.start = limit;
        self.buffer.clear();
        if !self.settled {
            self.grow(took, total - limit);
        }
        Ok(())
    }

    /// Double the chunk while chunks go out fast and memory is free (for
    /// both buffers when they overlap).
    fn grow(&mut self, took: Duration, remaining: u64) {
        let next = self.capacity.saturating_mul(2).min(MAX_CHUNK);
        if took >= CHUNK_TARGET || next <= self.capacity || remaining <= self.capacity as u64 {
            return;
        }
        let buffers = if self.overlap == Some(true) { 2 } else { 1 };
        let extra = ((next - self.capacity) * buffers) as u64;
        if let Some(extra) = crate::transfer::try_reserve_memory(extra) {
            self.reservations.push(extra);
            self.buffer
                .reserve_exact(next.saturating_sub(self.buffer.len()));
            if let Some(spare) = self.spare.as_mut() {
                spare.reserve_exact(next);
            }
            self.capacity = next;
        }
    }
}

impl Drop for Stream {
    /// A chunk still on its way keeps its memory reserved until it lands,
    /// without making the dropping writer wait for it.
    fn drop(&mut self) {
        let Some(flight) = self.inflight.take() else {
            return;
        };
        let reservations = std::mem::take(&mut self.reservations);
        let _ = std::thread::Builder::new()
            .name("drive-chunk-end".into())
            .spawn(move || {
                let _ = flight.join();
                drop(reservations);
            });
    }
}

/// One chunk (`data` at upload offset `start`) up to `limit`: starts the
/// upload session first if needed (409: an earlier attempt created the ID,
/// the bytes then only feed the MD5), then sends and requires the server
/// to confirm exactly `limit` (or completion at `total`).
fn send_chunk(
    backend: &GDriveBackend,
    object: &NewObject,
    (upload, settled): (&mut Option<Resumable>, &mut bool),
    (start, data): (u64, &[u8]),
    total: u64,
    limit: u64,
) -> io::Result<()> {
    if upload.is_none() && !*settled {
        match backend.start_new_upload(object, total) {
            Ok(started) => *upload = Some(started),
            Err(error) if http_status(&error) == Some(409) => *settled = true,
            Err(error) => return Err(error),
        }
    }
    if *settled {
        return Ok(());
    }
    let Some(upload) = upload.as_mut() else {
        return Err(io::Error::other("Drive-Upload-Sitzung fehlt"));
    };
    let mut window = Window { start, data };
    let sent = with_bearer(backend, |bearer| {
        upload.send_until(&mut window, limit, data.len(), bearer)
    })?;
    if limit == total || matches!(sent, Sent::Confirmed(next) if next == limit) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Drive bestätigte einen Upload-Abschnitt nicht wie gesendet",
    ))
}

/// The chunk still in memory, addressed by upload offset.
struct Window<'a> {
    start: u64,
    data: &'a [u8],
}

impl ChunkSource for Window<'_> {
    fn body(&mut self, offset: u64, len: usize) -> io::Result<Box<dyn Read + '_>> {
        let bytes = offset
            .checked_sub(self.start)
            .and_then(|from| usize::try_from(from).ok())
            .and_then(|from| Some(from..from.checked_add(len)?))
            .and_then(|range| self.data.get(range))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Drive verlangte Bytes, die nicht mehr im Puffer liegen",
                )
            })?;
        Ok(Box::new(bytes))
    }
}

/// Run `send` with this backend's token callbacks.
pub(super) fn with_bearer<T>(
    backend: &GDriveBackend,
    send: impl FnOnce(&mut Bearer<'_>) -> io::Result<T>,
) -> io::Result<T> {
    let mut get = || backend.bearer();
    let mut refresh = || backend.force_refresh_bearer();
    send(&mut Bearer {
        get: &mut get,
        refresh: &mut refresh,
    })
}
