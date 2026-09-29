//! Writing a new file to a remote target without ever replacing anything:
//! in a folder this job created, providers that publish complete objects only
//! (Drive) write it in one step (K6); otherwise the bytes go to a private
//! stage with a random name (no existence probe) and are published
//! create-only, under "Name (n)" when the name is taken. Stages that failed
//! are removed where the provider can prove they are ours, else reported (K17).
use super::super::engine_names::{base_name, next_numbered, parent_rel};
use super::super::engine_policy::ends_job_at_target;
use super::ops::{relabeled, At, OpError, Outcome};
use super::queue::FileWork;
use super::Engine;
use crate::vfs::Backend;
use std::io::{self, Write};

/// A 64-bit random name only collides with a foreign file created under that
/// exact name; a few attempts are pure caution.
const STAGE_ATTEMPTS: usize = 8;

/// Where a new remote file is written.
pub(crate) enum Destination {
    /// Published by `flush` (fresh object in a folder of this job).
    Fresh {
        path: String,
        writer: Box<dyn Write + Send>,
    },
    /// A private stage published afterwards.
    Staged {
        path: String,
        stage: String,
        writer: Box<dyn Write + Send>,
    },
}

impl Destination {
    pub(crate) fn writer(&mut self) -> &mut (dyn Write + Send) {
        match self {
            Destination::Fresh { writer, .. } | Destination::Staged { writer, .. } => {
                writer.as_mut()
            }
        }
    }
}

/// Opens where `file` goes on `target`: exactly `size` bytes when known
/// (`None` for exported documents, whose length shows only while reading).
pub(crate) fn open(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    size: Option<u64>,
    parent_created: bool,
) -> Result<Destination, OpError> {
    let path = engine.folders.path_of(&file.rel);
    if let Some(size) = size.filter(|_| parent_created && !engine.resume) {
        if let Some(writer) = target
            .open_write_fresh(&path, size)
            .map_err(OpError::target)?
        {
            return Ok(Destination::Fresh { path, writer });
        }
    }
    let (stage, writer) = open_stage(target, &path, size)?;
    Ok(Destination::Staged {
        path,
        stage,
        writer,
    })
}

/// A private stage for `destination`: random name, no probe, a new name when
/// that one exists (never adopting it). Engine jobs are copies whose sources
/// stay, so a stage of known size is not forced to stable storage before it
/// is published (spec decision 1); sync keeps its durable stages.
pub(crate) fn open_stage(
    target: &dyn Backend,
    destination: &str,
    size: Option<u64>,
) -> Result<(String, Box<dyn Write + Send>), OpError> {
    let mut last = None;
    for _ in 0..STAGE_ATTEMPTS {
        let stage = stage_name(destination);
        let opened = match size {
            Some(size) => target.open_write_copy_stage_unsynced(&stage, size),
            None => target.open_write_copy_stage(&stage),
        };
        match opened {
            Ok(writer) => return Ok((stage, writer)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(error),
            // A refusal stays one: the file waits for the peer (K13).
            Err(error) if crate::vfs::congestion_of(&error).is_some() => {
                return Err(OpError::target(error))
            }
            Err(error) => {
                return Err(OpError::target(io::Error::new(
                    error.kind(),
                    format!(
                        "Private Upload-Stufe „{stage}“ öffnen ({:?}): {error}; kein unsicherer Schreib-Fallback",
                        error.kind()
                    ),
                )))
            }
        }
    }
    Err(OpError::target(last.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::AlreadyExists, "Keine freie Upload-Stufe")
    })))
}

/// Gives up a destination before it was completed: nothing is published.
pub(crate) fn abandon(engine: &Engine<'_>, target: &dyn Backend, destination: Destination) {
    match destination {
        Destination::Fresh { writer, .. } => drop(writer),
        Destination::Staged { stage, writer, .. } => {
            drop(writer);
            discard(engine, target, &stage);
        }
    }
}

/// A private, unpredictable name next to `destination`.
pub(crate) fn stage_name(destination: &str) -> String {
    format!("{destination}.se-upload-{}", super::random_hex())
}

/// Completes a written destination: `flush` (which uploads spooled bytes
/// and fails unless all of them came), the source check, then publication.
pub(crate) fn complete(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    destination: Destination,
    verify: impl FnOnce() -> Result<(), OpError>,
) -> Result<Outcome, OpError> {
    match destination {
        Destination::Fresh { path, mut writer } => {
            if let Err(error) = verify() {
                // Dropping an unflushed fresh writer publishes nothing.
                drop(writer);
                return Err(error);
            }
            writer.flush().map_err(|error| {
                // A full, read-only or refusing target, or one that is too
                // busy, took nothing (the job ends, or the file waits);
                // anything else may have created the file.
                let refused =
                    ends_job_at_target(error.kind()) || crate::vfs::congestion_of(&error).is_some();
                let at = if refused { At::Target } else { At::Unknown };
                OpError::at(at, relabeled(&error, &format!("„{path}“ abschließen")))
            })?;
            Ok(Outcome::Done)
        }
        Destination::Staged {
            path,
            stage,
            mut writer,
        } => {
            let flushed = writer.flush();
            drop(writer);
            if let Err(error) = flushed.map_err(OpError::target).and_then(|()| verify()) {
                discard(engine, target, &stage);
                return Err(error);
            }
            if engine.stopped() {
                discard(engine, target, &stage);
                return Err(OpError::canceled());
            }
            publish(engine, target, file, &stage, path)
        }
    }
}

/// Create-only publication of `stage` as `path`, numbered when taken.
pub(crate) fn publish(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    stage: &str,
    path: String,
) -> Result<Outcome, OpError> {
    let top_level = parent_rel(&file.rel).is_none();
    let mut destination = path;
    let mut names = next_numbered(base_name(&destination));
    loop {
        match target.promote_copy_stage(stage, &destination) {
            Ok(()) => {
                if top_level {
                    engine
                        .folders
                        .record_alias(&file.rel, base_name(&destination));
                }
                return Ok(Outcome::Done);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if engine.resume {
                    let outcome = existing_same_size(target, &destination, file.size);
                    discard(engine, target, stage);
                    return outcome;
                }
                let Some(next) = names.next() else {
                    discard(engine, target, stage);
                    return Err(OpError::at(
                        At::Publish,
                        io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            format!("Kein freier Name für „{destination}“ (AlreadyExists)"),
                        ),
                    ));
                };
                let parent = super::super::engine_names::parent_path(&destination);
                destination = super::super::engine_names::join_rel(&parent, &next);
            }
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                discard(engine, target, stage);
                return Err(OpError::at(At::Publish, error));
            }
            // Refused outright (full, read-only, not ours to write): nothing
            // was published, and the job ends.
            Err(error) if ends_job_at_target(error.kind()) => {
                discard(engine, target, stage);
                return Err(OpError::target(error));
            }
            Err(error) => {
                return Err(OpError::at(
                    At::Unknown,
                    io::Error::new(
                        error.kind(),
                        format!(
                            "„{destination}“ veröffentlichen ({:?}): {error}; mögliche Upload-Stufe „{stage}“ bleibt erhalten",
                            error.kind()
                        ),
                    ),
                ))
            }
        }
    }
}

/// Resume (K8): an existing file of the same size counts as transferred,
/// any other is left alone and reported.
pub(crate) fn existing_same_size(
    target: &dyn Backend,
    path: &str,
    size: u64,
) -> Result<Outcome, OpError> {
    match target.stat(path) {
        Ok(meta) if !meta.is_dir && !meta.is_symlink && meta.size == size => Ok(Outcome::Skipped),
        Ok(_) => Err(OpError::at(At::Publish, different_size())),
        Err(error) => Err(OpError::at(At::Publish, error)),
    }
}

pub(crate) fn different_size() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Ziel existiert mit anderer Größe – nicht ersetzt",
    )
}

/// Removes a stage this job created and never published; reports it when
/// the provider cannot prove the name is still ours (K17).
pub(crate) fn discard(engine: &Engine<'_>, target: &dyn Backend, stage: &str) {
    if let Err(error) = target.discard_copy_stage(stage) {
        if error.kind() == io::ErrorKind::NotFound {
            return;
        }
        engine.issue(
            stage,
            &format!(
                "Private Upload-Stufe „{stage}“ wurde nicht automatisch gelöscht ({error}); die Quelle bleibt erhalten"
            ),
        );
    }
}
