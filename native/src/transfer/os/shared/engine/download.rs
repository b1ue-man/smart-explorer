//! Remote file → local folder: the bytes go to a private `.part` next to the
//! destination (kept across the one retry, which continues where it stopped
//! when the provider can read from an offset, K8), are checked against the
//! listing (length, MD5 or one stat), and are published without replacing
//! anything unless the user chose "overwrite". New files are not synced to
//! disk one by one; a replacement is.
use super::super::engine_names::{next_numbered, parent_rel};
use super::super::local_stage::{create_download_part, ensure_local_space};
use super::super::walk_listers::native;
use super::ops::{At, Carry, Meter, OpError, OpResult, Outcome};
use super::queue::FileWork;
use super::source::RemoteCheck;
use super::Engine;
use crate::types::Conflict;
use crate::vfs::Backend;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// A private partial download; removed unless it was published.
pub(crate) struct Part {
    path: PathBuf,
    file: Option<File>,
    written: u64,
    published: bool,
}

impl Part {
    pub(crate) fn create(destination: &Path) -> Result<Self, OpError> {
        let (path, file) = create_download_part(destination).map_err(OpError::target)?;
        Ok(Self {
            path,
            file: Some(file),
            written: 0,
            published: false,
        })
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<(), OpError> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| OpError::target(io::Error::other("Teildatei ist geschlossen")))?;
        file.write_all(bytes).map_err(OpError::target)?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    pub(crate) fn written(&self) -> u64 {
        self.written
    }

    /// Starts over from byte 0 (the provider cannot continue mid-file).
    fn restart(&mut self) -> Result<(), OpError> {
        if let Some(file) = self.file.as_mut() {
            file.set_len(0).map_err(OpError::target)?;
            file.seek(SeekFrom::Start(0)).map_err(OpError::target)?;
        }
        self.written = 0;
        Ok(())
    }

    fn close(&mut self, durable: bool) -> Result<(), OpError> {
        if let Some(file) = self.file.take() {
            if durable {
                file.sync_all().map_err(OpError::target)?;
            }
        }
        Ok(())
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        self.file.take();
        if !self.published {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub(super) fn download(
    engine: &Engine<'_>,
    source: &dyn Backend,
    file: &FileWork,
    carry: &mut Carry,
    meter: &Meter<'_>,
    buffer: &mut [u8],
) -> OpResult {
    let destination = native(&engine.folders.path_of(&file.rel));
    if let Some(outcome) = existing(engine, &destination, file.size)? {
        return Ok(outcome);
    }
    let expected = source
        .read_size(&file.source, file.size)
        .map_err(OpError::source)?;
    let mut part = match carry.part.take() {
        Some(part) => part,
        None => {
            ensure_local_space(&destination, expected.unwrap_or(file.size)).map_err(|message| {
                OpError::target(io::Error::new(io::ErrorKind::StorageFull, message))
            })?;
            Part::create(&destination)?
        }
    };
    let mut check = RemoteCheck::new(file, expected);
    let mut reader = match open_reader(source, file, &mut part, &mut check, buffer) {
        Ok(reader) => reader,
        Err(error) => {
            carry.part = Some(part);
            return Err(error);
        }
    };
    meter.credit(part.written());
    loop {
        if engine.stopped() {
            return Err(OpError::canceled());
        }
        let limit = check.limit(part.written(), buffer.len());
        let read = match reader.read(&mut buffer[..limit]) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                carry.part = Some(part);
                return Err(OpError::source(error));
            }
        };
        if read == 0 {
            break;
        }
        check.update(file, part.written(), &buffer[..read])?;
        part.write(&buffer[..read])?;
        meter.add(read as u64);
    }
    drop(reader);
    let read = part.written();
    check.finish(source, file, read)?;
    publish_local(engine, part, &destination, file)
}

/// Opens the source where the part stops: from its written length when the
/// provider can, else from the start with an emptied part.
fn open_reader(
    source: &dyn Backend,
    file: &FileWork,
    part: &mut Part,
    check: &mut RemoteCheck,
    buffer: &mut [u8],
) -> Result<Box<dyn Read + Send>, OpError> {
    if part.written() > 0 {
        let resumed = source
            .open_read_at(&file.source, file.id.as_deref(), part.written())
            .map_err(OpError::source)?;
        if let Some(reader) = resumed {
            if check.hashing() {
                rehash(part, check, file, buffer)?;
            }
            return Ok(reader);
        }
        part.restart()?;
    }
    source
        .open_read_id(&file.source, file.id.as_deref())
        .map_err(OpError::source)
}

/// Feeds the bytes already in the part to the change check (MD5).
fn rehash(
    part: &Part,
    check: &mut RemoteCheck,
    file: &FileWork,
    buffer: &mut [u8],
) -> Result<(), OpError> {
    let mut reader = File::open(&part.path).map_err(OpError::target)?;
    let mut offset = 0u64;
    while offset < part.written() {
        let wanted = (part.written() - offset).min(buffer.len() as u64) as usize;
        let read = reader
            .read(&mut buffer[..wanted])
            .map_err(OpError::target)?;
        if read == 0 {
            break;
        }
        check.update(file, offset, &buffer[..read])?;
        offset += read as u64;
    }
    Ok(())
}

/// Decides before any transfer whether an existing destination is kept:
/// "skip" and "transfer missing files" leave it alone.
pub(super) fn existing(
    engine: &Engine<'_>,
    destination: &Path,
    size: u64,
) -> Result<Option<Outcome>, OpError> {
    if !(engine.resume || engine.view.conflict == Conflict::Skip) {
        return Ok(None);
    }
    match std::fs::symlink_metadata(destination) {
        Ok(metadata) if engine.resume => {
            if metadata.is_file() && metadata.len() == size {
                Ok(Some(Outcome::Skipped))
            } else {
                Err(OpError::at(At::Publish, super::publish::different_size()))
            }
        }
        Ok(_) => Ok(Some(Outcome::Skipped)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(OpError::target(error)),
    }
}

/// Publishes a complete part under the job's conflict policy.
pub(super) fn publish_local(
    engine: &Engine<'_>,
    mut part: Part,
    destination: &Path,
    file: &FileWork,
) -> OpResult {
    let overwrite = engine.view.conflict == Conflict::Overwrite && !engine.resume;
    part.close(overwrite)?;
    if overwrite {
        refuse_folder_or_link(destination)?;
        super::super::platform::replace_file_atomic(&part.path, destination)
            .map_err(|error| OpError::at(At::Publish, error))?;
        part.published = true;
        return Ok(Outcome::Done);
    }
    let mut target = destination.to_path_buf();
    let mut names = next_numbered(&file_name(destination));
    loop {
        match crate::vfs::promote_local_copy(&part.path, &target) {
            Ok(()) => {
                part.published = true;
                if parent_rel(&file.rel).is_none() {
                    engine.folders.record_alias(&file.rel, &file_name(&target));
                }
                return Ok(Outcome::Done);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if engine.resume {
                    return existing(engine, destination, part.written())?
                        .ok_or_else(|| OpError::at(At::Publish, super::publish::different_size()));
                }
                if engine.view.conflict == Conflict::Skip {
                    return Ok(Outcome::Skipped);
                }
                target = next_free(destination, &mut names)?;
            }
            Err(error) => {
                return Err(OpError::at(
                    At::Publish,
                    io::Error::new(
                        error.kind(),
                        format!(
                            "„{}“ ohne Ersetzen veröffentlichen ({:?}): {error}",
                            destination.display(),
                            error.kind()
                        ),
                    ),
                ))
            }
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The next numbered sibling that does not exist yet.
fn next_free(
    destination: &Path,
    names: &mut impl Iterator<Item = String>,
) -> Result<PathBuf, OpError> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    for name in names {
        let candidate = parent.join(name);
        match std::fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => return Err(OpError::at(At::Publish, error)),
            Ok(_) => {}
        }
    }
    Err(OpError::at(
        At::Publish,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "Kein freier Name für „{}“ (AlreadyExists)",
                destination.display()
            ),
        ),
    ))
}

fn refuse_folder_or_link(destination: &Path) -> Result<(), OpError> {
    match std::fs::symlink_metadata(destination) {
        Ok(metadata)
            if metadata.is_dir()
                || crate::local_access::metadata_is_link_like(destination, &metadata) =>
        {
            Err(OpError::at(
                At::Publish,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "Ziel ist ein Ordner oder Link und wird nicht ersetzt: {}",
                        destination.display()
                    ),
                ),
            ))
        }
        _ => Ok(()),
    }
}
