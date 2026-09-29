//! A resumable upload streamed chunk by chunk from memory: each full chunk
//! goes out as soon as the next bytes arrive, only that one chunk is held
//! (its memory reserved, plan K7), and fast chunks double up to a bound.
use super::new_object::NewObject;
use super::overload::http_status;
use super::resumable::{self, Bearer, ChunkSource, Resumable, Sent};
use super::GDriveBackend;
use crate::transfer::MemoryReservation;
use std::io::{self, Read};
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
    reservations: Vec<MemoryReservation>,
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
                self.send(backend, object, total, end)?;
            }
        }
        Ok(())
    }

    /// Send the buffered bytes up to `limit` (`total` completes the upload).
    pub(super) fn send(
        &mut self,
        backend: &GDriveBackend,
        object: &NewObject,
        total: u64,
        limit: u64,
    ) -> io::Result<()> {
        if self.upload.is_none() && !self.settled {
            match backend.start_new_upload(object, total) {
                Ok(upload) => self.upload = Some(upload),
                Err(error) if http_status(&error) == Some(409) => self.settled = true,
                Err(error) => return Err(error),
            }
        }
        if self.settled {
            self.start = limit;
            self.buffer.clear();
            return Ok(());
        }
        let Some(upload) = self.upload.as_mut() else {
            return Err(io::Error::other("Drive-Upload-Sitzung fehlt"));
        };
        let mut window = Window {
            start: self.start,
            data: &self.buffer,
        };
        let started = Instant::now();
        let sent = with_bearer(backend, |bearer| {
            upload.send_until(&mut window, limit, self.buffer.len(), bearer)
        })?;
        if limit == total {
            return Ok(());
        }
        if !matches!(sent, Sent::Confirmed(next) if next == limit) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive bestätigte einen Upload-Abschnitt nicht wie gesendet",
            ));
        }
        let took = started.elapsed();
        self.start = limit;
        self.buffer.clear();
        self.grow(took, total - limit);
        Ok(())
    }

    /// Double the chunk while chunks go out fast and memory is free.
    fn grow(&mut self, took: Duration, remaining: u64) {
        let next = self.capacity.saturating_mul(2).min(MAX_CHUNK);
        if took >= CHUNK_TARGET || next <= self.capacity || remaining <= self.capacity as u64 {
            return;
        }
        if let Some(extra) = crate::transfer::try_reserve_memory((next - self.capacity) as u64) {
            self.reservations.push(extra);
            self.buffer.reserve_exact(next);
            self.capacity = next;
        }
    }
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
