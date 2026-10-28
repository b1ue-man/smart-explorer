//! A segmented stage over Nextcloud's chunked upload (G3-12, 2026-10-28):
//! the ranges of one large file go up side by side as numbered parts into a
//! private upload folder, each its own `PUT` on its own pooled connection,
//! and `finish` assembles them with one `MOVE` of `.file` into the stage
//! (chunked_upload.rs has the protocol rules). The stage then is an
//! ordinary copy stage: the engine publishes it with `MOVE` and
//! `Overwrite: F` like a streamed one, so nothing appears under the
//! destination name before that and nothing is ever replaced.
//!
//! Plain WebDAV is unchanged: a server is asked once per connection with the
//! first upload folder (`MKCOL` with `Destination`); a refusal is
//! remembered and its files keep the single streamed `PUT`.
use super::chunked_upload::{
    part_name, part_number, pieces, remember_support, support, unit, upload_home, MIN_PART,
};
use super::core_impl::{request_err, WebdavBackend};
use super::stream_put::{PutHeaders, StreamPut};
use crate::vfs::{Backend, SegmentedStage, VfsResult};
use std::collections::{BTreeMap, HashSet};
use std::io::{self, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Part numbers handed out and ranges whose parts all were confirmed.
#[derive(Default)]
struct Parts {
    numbers: HashSet<u64>,
    /// Offset → length of every confirmed range.
    done: BTreeMap<u64, u64>,
}

struct Shared {
    backend: WebdavBackend,
    /// Path of the upload folder (`.../uploads/<user>/se-<hex>`).
    folder: String,
    /// URL of the stage: every request of the upload names it.
    destination: String,
    size: u64,
    parts: Mutex<Parts>,
}

impl Shared {
    fn parts(&self) -> MutexGuard<'_, Parts> {
        self.parts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn part_url(&self, number: u64) -> String {
        let path = format!("{}/{}", self.folder, part_name(number));
        self.backend.url_for(&path)
    }

    /// Removes the upload folder; unpooled like every DELETE (ureq would
    /// replay it). Failures are not reported: Nextcloud expires the folder.
    fn remove_folder(&self) {
        let mut url = self.backend.url_for(&self.folder);
        url.push('/');
        let request = self.backend.mutation_agent.request("DELETE", &url);
        let _ = self.backend.auth_req(request).call();
    }
}

pub(super) struct ChunkedStage {
    stage: String,
    unit: u64,
    shared: Arc<Shared>,
}

/// How the first upload folder of a connection was answered.
enum Folder {
    Created,
    /// The server has no chunked upload here (any other 4xx, 501).
    Refused(io::Error),
    Failed(io::Error),
}

fn make_folder(backend: &WebdavBackend, folder: &str, destination: &str) -> Folder {
    let mut url = backend.url_for(folder);
    url.push('/');
    let request = backend
        .write_agent
        .request("MKCOL", &url)
        .set("Destination", destination);
    match backend.auth_req(request).call() {
        Ok(response) if (200..300).contains(&response.status()) && response.status() != 207 => {
            Folder::Created
        }
        Ok(response) => Folder::Refused(io::Error::other(format!(
            "WebDAV MKCOL des Upload-Ordners: HTTP-Status {}",
            response.status()
        ))),
        Err(error @ ureq::Error::Status(401 | 413 | 429 | 507, _)) => {
            Folder::Failed(request_err(error))
        }
        Err(error @ ureq::Error::Status(400..=499 | 501, _)) => Folder::Refused(request_err(error)),
        Err(error) => Folder::Failed(request_err(error)),
    }
}

fn random_hex() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    if let Err(error) = getrandom::getrandom(&mut bytes) {
        return Err(io::Error::other(error.to_string()));
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The chunked stage for `stage` (`size` bytes), or `None` where this
/// server or this file takes the single `PUT`.
pub(super) fn open(
    backend: &WebdavBackend,
    stage: &str,
    size: u64,
) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
    let (Some(home), Some(unit)) = (upload_home(stage), unit(size)) else {
        return Ok(None);
    };
    let key = format!("{}|{home}", backend.namespace_identity());
    let known = support(&key);
    if known == Some(false) {
        return Ok(None);
    }
    let folder = format!("{home}/se-{}", random_hex()?);
    let destination = backend.url_for(stage);
    match make_folder(backend, &folder, &destination) {
        Folder::Created => remember_support(&key, true),
        Folder::Refused(_) if known.is_none() => {
            remember_support(&key, false);
            return Ok(None);
        }
        Folder::Refused(error) | Folder::Failed(error)
            if known == Some(true) || crate::vfs::congestion_of(&error).is_some() =>
        {
            return Err(error)
        }
        // Not known yet and no clear answer: this file streams, the next
        // one asks again.
        Folder::Refused(_) | Folder::Failed(_) => return Ok(None),
    }
    Ok(Some(Box::new(ChunkedStage {
        stage: stage.to_string(),
        unit,
        shared: Arc::new(Shared {
            backend: backend.clone(),
            folder,
            destination,
            size,
            parts: Mutex::new(Parts::default()),
        }),
    })))
}

impl ChunkedStage {
    /// Every byte of the file lies in exactly one confirmed range.
    fn complete(&self) -> bool {
        let parts = self.shared.parts();
        let mut next = 0;
        for (&offset, &len) in &parts.done {
            if offset != next {
                return false;
            }
            next = offset + len;
        }
        next == self.shared.size
    }

    /// Assembles the parts into the stage and checks its length.
    fn assemble(&self) -> VfsResult<()> {
        let backend = &self.shared.backend;
        let url = backend.url_for(&format!("{}/.file", self.shared.folder));
        let request = backend
            .write_agent
            .request("MOVE", &url)
            .set("Destination", &self.shared.destination)
            .set("Overwrite", "F")
            .set("OC-Total-Length", &self.shared.size.to_string());
        backend.mutation(request, "MOVE (Zusammensetzen)")?;
        let meta = backend.stat(&self.stage)?;
        if meta.is_dir || meta.size != self.shared.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Zusammengesetzte Stufe {} hat {} statt {} Bytes",
                    self.stage, meta.size, self.shared.size
                ),
            ));
        }
        Ok(())
    }
}

impl SegmentedStage for ChunkedStage {
    fn stage_path(&self) -> &str {
        &self.stage
    }

    fn segment(&self, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>> {
        let size = self.shared.size;
        let end = offset.checked_add(len).filter(|end| *end <= size);
        if len == 0 || end.is_none() || (len < MIN_PART && end != Some(size)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Bereich {offset}+{len} passt nicht in den Chunk-Upload von {size} Bytes"),
            ));
        }
        let pieces = pieces(offset, len);
        let numbers: Vec<u64> = pieces
            .iter()
            .map(|(start, _)| part_number(*start, self.unit))
            .collect();
        {
            let mut parts = self.shared.parts();
            if numbers.iter().any(|number| parts.numbers.contains(number)) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Teilnummer für Bereich {offset}+{len} ist schon vergeben"),
                ));
            }
            parts.numbers.extend(numbers.iter().copied());
        }
        Ok(Box::new(PartWriter {
            shared: self.shared.clone(),
            offset,
            len,
            pieces: pieces.into_iter().zip(numbers).collect(),
            index: 0,
            current: None,
            filled: 0,
            failed: false,
        }))
    }

    fn finish(self: Box<Self>) -> VfsResult<()> {
        if !self.complete() {
            self.shared.remove_folder();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Nicht alle Teile von {} sind bestätigt", self.stage),
            ));
        }
        let assembled = self.assemble();
        if assembled.is_err() {
            // A refused MOVE leaves the folder; an assembled one is gone.
            self.shared.remove_folder();
        } else {
            // Published like a stage whose PUT was seen committed: the MOVE
            // created it and its length was just read back.
            super::put_stages::committed(&self.shared.destination);
        }
        assembled
    }

    fn abort(self: Box<Self>) {
        self.shared.remove_folder();
    }
}

fn failed_range() -> io::Error {
    io::Error::other("Chunk-Upload dieses Bereichs ist fehlgeschlagen")
}

/// The writer of one range: its parts one after the other, each a streamed
/// `PUT` of known length that ureq never replays.
struct PartWriter {
    shared: Arc<Shared>,
    offset: u64,
    len: u64,
    /// `((offset, len), number)` of every part of the range.
    pieces: Vec<((u64, u64), u64)>,
    index: usize,
    current: Option<StreamPut>,
    /// Bytes in the current part.
    filled: u64,
    failed: bool,
}

impl PartWriter {
    fn start(&mut self) -> io::Result<()> {
        let ((_, len), number) = self.pieces[self.index];
        let headers = PutHeaders {
            part: true,
            extra: vec![
                ("Destination", self.shared.destination.clone()),
                ("OC-Total-Length", self.shared.size.to_string()),
            ],
            ..PutHeaders::default()
        };
        let backend = &self.shared.backend;
        self.current = Some(StreamPut::start_with(
            backend.write_agent.clone(),
            self.shared.part_url(number),
            backend.auth.clone(),
            len,
            headers,
        )?);
        self.filled = 0;
        Ok(())
    }

    /// Completes the current part: its `PUT` must be confirmed.
    fn close_part(&mut self) -> io::Result<()> {
        if let Some(mut part) = self.current.take() {
            part.flush()?;
            self.index += 1;
        }
        Ok(())
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        self.failed = true;
        self.current = None;
        error
    }
}

impl Write for PartWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.failed {
            return Err(failed_range());
        }
        if data.is_empty() {
            return Ok(0);
        }
        if self.index >= self.pieces.len() {
            return Err(self.fail(io::Error::new(
                io::ErrorKind::InvalidData,
                "Mehr Bytes als der Bereich fasst",
            )));
        }
        if self.current.is_none() {
            if let Err(error) = self.start() {
                return Err(self.fail(error));
            }
        }
        let ((_, len), _) = self.pieces[self.index];
        let take = (len - self.filled).min(data.len() as u64) as usize;
        let written = match self.current.as_mut().map(|part| part.write(&data[..take])) {
            Some(Ok(written)) => written,
            Some(Err(error)) => return Err(self.fail(error)),
            None => return Err(self.fail(io::Error::other("Kein offener Teil"))),
        };
        self.filled += written as u64;
        if self.filled == len {
            if let Err(error) = self.close_part() {
                return Err(self.fail(error));
            }
        }
        Ok(written)
    }

    /// Succeeds only when every part of the range was confirmed.
    fn flush(&mut self) -> io::Result<()> {
        if self.failed {
            return Err(failed_range());
        }
        if self.index < self.pieces.len() || self.current.is_some() {
            return Err(self.fail(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "Bereich {}+{} des Chunk-Uploads ist unvollständig",
                    self.offset, self.len
                ),
            )));
        }
        self.shared.parts().done.insert(self.offset, self.len);
        Ok(())
    }
}
