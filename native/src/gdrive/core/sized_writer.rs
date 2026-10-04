//! Writers that know their final size (plan C4, K6). `open_write_fresh`
//! creates a new file in a folder the transfer created itself with a single
//! write request (multipart up to 5 MB, else a resumable session under a
//! pre-generated ID): no probe, no stage, no promotion. The sized copy stage
//! creates its private object the same way. Both stream instead of spooling to
//! a temporary file while the transfer memory budget allows.
use super::chunk_stream::{with_bearer, Stream, FIRST_CHUNK};
use super::core::{norm, split_parent};
use super::new_object::NewObject;
use super::resumable;
use super::GDriveBackend;
use crate::transfer::MemoryReservation;
use crate::vfs::VfsResult;
use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};

/// Google documents multipart uploads for "a small file (5 MB or less)"
/// (ref §1); the decimal reading is the smaller one.
const MULTIPART_LIMIT: u64 = 5_000_000;

#[derive(Clone, Copy)]
enum Purpose {
    /// New file in a folder the transfer created (`open_write_fresh`).
    Fresh,
    /// Private copy stage (`open_write_copy_stage_sized`).
    Stage,
}

pub(super) fn open_fresh(
    backend: &GDriveBackend,
    path: &str,
    size: u64,
) -> VfsResult<Box<dyn Write + Send>> {
    Ok(Box::new(SizedWriter::open(
        backend,
        path,
        size,
        Purpose::Fresh,
        None,
    )?))
}

pub(super) fn open_stage(
    backend: &GDriveBackend,
    path: &str,
    size: u64,
) -> VfsResult<Box<dyn Write + Send>> {
    Ok(Box::new(SizedWriter::open(
        backend,
        path,
        size,
        Purpose::Stage,
        None,
    )?))
}

pub(super) fn open_stage_timed(
    backend: &GDriveBackend,
    path: &str,
    size: u64,
    mtime_ms: i64,
) -> VfsResult<Box<dyn Write + Send>> {
    Ok(Box::new(SizedWriter::open(
        backend,
        path,
        size,
        Purpose::Stage,
        Some(mtime_ms),
    )?))
}

struct SizedWriter {
    backend: GDriveBackend,
    purpose: Purpose,
    key: String,
    object: NewObject,
    size: u64,
    received: u64,
    md5: md5::Context,
    body: Body,
    /// This writer reserved a new ID (not one an earlier attempt left).
    new_id: bool,
    /// A request that can create the object went out.
    create_sent: bool,
    state: State,
}

enum State {
    Open,
    Committed,
    Failed(io::ErrorKind, String),
}

enum Body {
    /// The whole content, sent as one multipart request.
    Memory {
        data: Vec<u8>,
        _reservation: MemoryReservation,
    },
    /// Memory was short: content on disk, as the other writers spool.
    Spool(File),
    /// Resumable upload, chunk by chunk from memory.
    Stream(Stream),
}

impl SizedWriter {
    fn open(
        backend: &GDriveBackend,
        path: &str,
        size: u64,
        purpose: Purpose,
        mtime_ms: Option<i64>,
    ) -> VfsResult<Self> {
        let key = norm(path);
        if key.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Drive-Upload braucht einen Dateinamen unterhalb der Wurzel",
            ));
        }
        let (parent, name) = split_parent(&key);
        let title = super::names::decode(name)?;
        let (parent_id, id, new_id) = match purpose {
            // The transfer created this folder, so the name is free there: no
            // probe. An ID an earlier, possibly committed attempt reserved is
            // kept, so a retry gets 409 instead of making a second file.
            Purpose::Fresh => {
                let parent_id = backend.resolve(&parent)?;
                if backend.bound_folder(&parent_id, name)?.is_some() {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists,
                        "Drive file destination is a reserved folder locator"));
                }
                let (id, new_id) = claim_upload_id(backend, &key)?;
                (parent_id, id, new_id)
            }
            // A stage owns a new ID, never a pending or cached one of its path.
            Purpose::Stage => {
                let parent_id = backend.ensure_dir(&parent)?;
                if backend.bound_folder(&parent_id, name)?.is_some() {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists,
                        "Drive stage name is a reserved folder locator"));
                }
                let id = backend.take_generated_id()?;
                backend.own_stage(&key, &id)?;
                (parent_id, id, true)
            }
        };
        let body = Body::for_size(size)?;
        Ok(Self {
            backend: backend.clone(),
            purpose,
            key,
            object: NewObject {
                id,
                parent_id,
                title,
                declare_binary: matches!(purpose, Purpose::Stage),
                mtime_ms,
            },
            size,
            received: 0,
            md5: md5::Context::new(),
            body,
            new_id,
            create_sent: false,
            state: State::Open,
        })
    }

    fn check_open(&self) -> io::Result<()> {
        match &self.state {
            State::Open => Ok(()),
            State::Committed => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Drive-Upload ist bereits abgeschlossen",
            )),
            State::Failed(kind, message) => Err(io::Error::new(*kind, message.clone())),
        }
    }

    /// Freeze the writer after an error; later calls repeat it.
    fn fail(&mut self, error: io::Error) -> io::Error {
        self.state = State::Failed(error.kind(), error.to_string());
        error
    }

    fn commit(&mut self) -> io::Result<()> {
        let md5 = format!("{:x}", self.md5.clone().compute());
        // One upload per path at a time, like every writer of this backend.
        let _path = self.backend.upload_path_guard(&self.key)?;
        self.create_sent = true;
        let backend = &self.backend;
        let object = &self.object;
        let mime = match &mut self.body {
            Body::Memory { data, .. } => {
                backend.create_multipart(object, data.as_slice(), self.size, &md5)?
            }
            Body::Spool(file) => {
                file.flush()?;
                file.seek(SeekFrom::Start(0))?;
                if self.size <= MULTIPART_LIMIT {
                    backend.create_multipart(object, &mut *file, self.size, &md5)?
                } else {
                    let sent =
                        backend
                            .start_new_upload(object, self.size)
                            .and_then(|mut upload| {
                                with_bearer(backend, |bearer| {
                                    resumable::send_spool(&mut upload, file, bearer)
                                })
                            });
                    // Whatever the last answer was, the reserved ID decides
                    // (an earlier attempt may have committed it: 409).
                    let failure = sent.err().unwrap_or_else(|| unconfirmed(object));
                    backend.verify_created(object, self.size, Some(&md5), failure)?
                }
            }
            Body::Stream(stream) => {
                let sent = stream.send(backend, object, self.size, self.size);
                let failure = sent.err().unwrap_or_else(|| unconfirmed(object));
                backend.verify_created(object, self.size, Some(&md5), failure)?
            }
        };
        if matches!(self.purpose, Purpose::Stage) {
            // Like the spooled stage: the stage name must lead to exactly this
            // ID, else the caller picks another name (and discards this one).
            let objects = backend.named_objects(&object.parent_id, &object.title)?;
            if objects.len() != 1 || objects[0].id != object.id {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "Der Drive-Stufenname führt nicht eindeutig zur eigenen ID {}",
                        object.id
                    ),
                ));
            }
        }
        backend.remember_path(&self.key, &object.id, Some(&mime))?;
        if matches!(self.purpose, Purpose::Fresh) {
            let mut pending = backend.pending_upload_ids_guard()?;
            if pending.get(&self.key) == Some(&object.id) {
                pending.remove(&self.key);
            }
        }
        backend.persist_path_cache();
        Ok(())
    }
}

impl Write for SizedWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.check_open()?;
        let incoming = data.len() as u64;
        if incoming > self.size - self.received {
            let error = io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Drive-Upload erhielt mehr als die angekündigten {} Bytes",
                    self.size
                ),
            );
            return Err(self.fail(error));
        }
        let result = match &mut self.body {
            Body::Memory { data: buffer, .. } => {
                buffer.extend_from_slice(data);
                Ok(())
            }
            Body::Spool(file) => file.write_all(data),
            Body::Stream(stream) => stream.push(&self.backend, &self.object, self.size, data),
        };
        if let Err(error) = result {
            return Err(self.fail(error));
        }
        self.md5.consume(data);
        self.received += incoming;
        Ok(data.len())
    }

    /// Publish: fails unless exactly the announced size arrived.
    fn flush(&mut self) -> io::Result<()> {
        match &self.state {
            State::Committed => return Ok(()),
            State::Failed(..) => return self.check_open(),
            State::Open => {}
        }
        if self.received != self.size {
            let error = io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "Drive-Upload unvollständig: {} von {} Bytes",
                    self.received, self.size
                ),
            );
            return Err(self.fail(error));
        }
        match self.commit() {
            Ok(()) => {
                self.state = State::Committed;
                Ok(())
            }
            Err(error) => Err(self.fail(error)),
        }
    }
}

impl Drop for SizedWriter {
    fn drop(&mut self) {
        // Drop is abort: nothing is published without flush. An ID this
        // writer reserved is released unless a create went out (then a retry
        // must meet the same ID).
        if matches!(self.state, State::Committed) || self.create_sent || !self.new_id {
            return;
        }
        let owner = match self.purpose {
            Purpose::Fresh => self.backend.pending_upload_ids_guard(),
            Purpose::Stage => self.backend.owned_stages_guard(),
        };
        if let Ok(mut owner) = owner {
            if owner.get(&self.key) == Some(&self.object.id) {
                owner.remove(&self.key);
            }
        }
    }
}

impl Body {
    fn for_size(size: u64) -> io::Result<Self> {
        if size <= MULTIPART_LIMIT {
            if let Some(reservation) = crate::transfer::try_reserve_memory(size) {
                return Ok(Body::Memory {
                    // At most 5 MB.
                    data: Vec::with_capacity(size as usize),
                    _reservation: reservation,
                });
            }
            return Ok(Body::Spool(tempfile::tempfile()?));
        }
        let chunk = usize::try_from(size).map_or(FIRST_CHUNK, |size| size.min(FIRST_CHUNK));
        match crate::transfer::try_reserve_memory(chunk as u64) {
            Some(reservation) => Ok(Body::Stream(Stream::new(chunk, reservation))),
            // Waiting for memory while holding a transfer permit is not
            // allowed (K2): spool to disk instead.
            None => Ok(Body::Spool(tempfile::tempfile()?)),
        }
    }
}

/// Reserve the ID for a new file at `key`: the one an earlier attempt left
/// pending, else a new one (true) that stays pending until verified.
fn claim_upload_id(backend: &GDriveBackend, key: &str) -> io::Result<(String, bool)> {
    if let Some(id) = backend.pending_upload_ids_guard()?.get(key).cloned() {
        return Ok((id, false));
    }
    let id = backend.take_generated_id()?;
    let mut pending = backend.pending_upload_ids_guard()?;
    if let Some(existing) = pending.get(key) {
        return Ok((existing.clone(), false));
    }
    pending.insert(key.to_string(), id.clone());
    Ok((id, true))
}

fn unconfirmed(object: &NewObject) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "Drive meldete den Upload als fertig, kennt die ID {} aber nicht",
            object.id
        ),
    )
}
