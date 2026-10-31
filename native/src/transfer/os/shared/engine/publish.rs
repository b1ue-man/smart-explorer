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
    /// Created under its final name in one write request, the name having
    /// been seen absent (G3-08b); the provider proves it after `flush`.
    Absent {
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
            Destination::Fresh { writer, .. }
            | Destination::Absent { writer, .. }
            | Destination::Staged { writer, .. } => writer.as_mut(),
        }
    }
}

/// Opens where `file` goes on `target`: exactly `size` bytes when known
/// (`None` for exported documents, whose length shows only while reading).
/// `known_absent`: the job saw the destination name absent (a top-level name
/// claimed against the target listing, `FolderRegister::known_absent`).
pub(crate) fn open(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    size: Option<u64>,
    parent_created: bool,
    known_absent: bool,
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
    let one_request = known_absent
        && !engine.resume
        && !file.absent_refused
        && engine.conflict_of(file) == crate::types::Conflict::Rename;
    if let Some(size) = size.filter(|_| one_request) {
        match crate::vfs::open_write_absent(target, &path, size) {
            Ok(Some(writer)) => return Ok(Destination::Absent { path, writer }),
            // Not offered, or the name is taken already: the staged path
            // below numbers it.
            Ok(None) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(OpError::target(error)),
        }
    }
    // A replacement of an existing file is staged durably, as every other
    // replacement is (sync, mounts); a new copy keeps the unsynced stage.
    let (stage, writer) =
        open_stage_for(target, &path, size, file.replacing.is_some(), file.mtime_ms)?;
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
    open_stage_for(target, destination, size, false, 0)
}

/// `open_stage`; `durable`: the stage replaces an existing file, so it is
/// not the unsynced stage of a copy that keeps its source. `mtime_ms`: the
/// source's time (0 = unknown) for providers that take it only while
/// uploading (`publish_time::at_upload`).
pub(crate) fn open_stage_for(
    target: &dyn Backend,
    destination: &str,
    size: Option<u64>,
    durable: bool,
    mtime_ms: i64,
) -> Result<(String, Box<dyn Write + Send>), OpError> {
    let mut last = None;
    for _ in 0..STAGE_ATTEMPTS {
        let stage = stage_name(destination);
        let opened = match size {
            Some(size) if super::publish_time::at_upload(target, mtime_ms) => {
                crate::vfs::open_write_copy_stage_timed(target, &stage, size, mtime_ms)
            }
            Some(size) if durable => target.open_write_copy_stage_sized(&stage, size),
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
        // Dropping an unflushed writer publishes nothing.
        Destination::Fresh { writer, .. } | Destination::Absent { writer, .. } => drop(writer),
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
            if let Err(error) = super::file_recovery::prepare_direct(engine, file, &path) {
                super::file_recovery::preparation_failed(engine, file)
                    .map_err(|failure| OpError::at(At::Unknown, failure))?;
                return Err(OpError::target(error));
            }
            if let Err(error) = writer.flush() {
                // A full, read-only or refusing target, or one that is too
                // busy, took nothing (the job ends, or the file waits);
                // anything else may have created the file.
                let refused =
                    ends_job_at_target(error.kind()) || crate::vfs::congestion_of(&error).is_some();
                let at = if refused { At::Target } else { At::Unknown };
                if refused {
                    super::file_recovery::unpublished(engine, file)
                        .map_err(|error| OpError::at(At::Unknown, error))?;
                }
                return Err(OpError::at(
                    at,
                    relabeled(&error, &format!("„{path}“ abschließen")),
                ));
            }
            drop(writer);
            super::file_recovery::after(engine, file, &path)
                .map_err(|error| OpError::at(At::Unknown, error))?;
            Ok(Outcome::Done)
        }
        Destination::Absent { path, writer } => complete_absent(engine, file, path, writer, verify),
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

/// Completes a one-request publication under a name seen absent (G3-08b):
/// like a fresh object, but the receipt records the absence the listing
/// showed instead of asking again, and a name taken meanwhile (the provider
/// withdrew its own object) sends the file to the staged path at once.
fn complete_absent(
    engine: &Engine<'_>,
    file: &FileWork,
    path: String,
    mut writer: Box<dyn Write + Send>,
    verify: impl FnOnce() -> Result<(), OpError>,
) -> Result<Outcome, OpError> {
    if let Err(error) = verify() {
        drop(writer);
        return Err(error);
    }
    if let Err(error) =
        super::file_recovery::prepare_direct_known(engine, file, &path, None, Some(false))
    {
        super::file_recovery::preparation_failed(engine, file)
            .map_err(|failure| OpError::at(At::Unknown, failure))?;
        return Err(OpError::target(error));
    }
    if let Err(error) = writer.flush() {
        drop(writer);
        if error.kind() == io::ErrorKind::AlreadyExists {
            super::file_recovery::unpublished(engine, file)
                .map_err(|error| OpError::at(At::Unknown, error))?;
            return Err(OpError::absent_taken(relabeled(
                &error,
                &format!("„{path}“ ist inzwischen belegt"),
            )));
        }
        let refused =
            ends_job_at_target(error.kind()) || crate::vfs::congestion_of(&error).is_some();
        let at = if refused { At::Target } else { At::Unknown };
        if refused {
            super::file_recovery::unpublished(engine, file)
                .map_err(|error| OpError::at(At::Unknown, error))?;
        }
        return Err(OpError::at(
            at,
            relabeled(&error, &format!("„{path}“ abschließen")),
        ));
    }
    drop(writer);
    super::file_recovery::after(engine, file, &path)
        .map_err(|error| OpError::at(At::Unknown, error))?;
    Ok(Outcome::Done)
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
    // The stage gets the source's time before anything is recorded or
    // published; evidence the peer gave about the stage is then out of date.
    let timed = match super::publish_time::carry(engine, target, file, stage) {
        Ok(applied) => applied,
        Err(error) => {
            discard(engine, target, stage);
            return Err(error);
        }
    };
    // The file rule "neuere ersetzen ältere" replaces an older target in one
    // server-side step bound to the object it saw; a target that cannot, or
    // that changed, keeps both below (`replacement_refused`).
    let mut replacing = file.replacing.is_some() && !engine.resume;
    let mut destination = path;
    let mut names = next_numbered(base_name(&destination));
    // A peer with transfer evidence reported the stage when the write was
    // committed, and whether the file it is meant for existed then: the
    // receipt needs no request of its own (a numbered name is asked about).
    let known = crate::vfs::take_stage_evidence(target, stage).filter(|_| !timed);
    let stage_known = known
        .as_ref()
        .map(|known| super::batch_put::host_evidence(known.stage.clone()));
    loop {
        let existed = known
            .as_ref()
            .filter(|known| known.destination.as_deref() == Some(destination.as_str()))
            .and_then(|known| known.existed)
            .or(replacing.then_some(true));
        let prepared = super::file_recovery::before_known(
            engine,
            file,
            stage,
            &destination,
            None,
            stage_known.as_ref(),
            existed,
        );
        if let Err(error) = prepared {
            if let Err(failure) = super::file_recovery::preparation_failed(engine, file) {
                return Err(OpError::at(At::Unknown, io::Error::other(format!("Publikationsvorbereitung fehlgeschlagen: {error}; Nichtpublikation unbestätigt: {failure}; eigene Stage bleibt erhalten: {stage}"))));
            }
            discard(engine, target, stage);
            return Err(OpError::target(error));
        }
        let promoted = if replacing {
            let id = file.replacing.as_ref().and_then(|id| id.as_deref());
            target
                .promote_staged_to_id(stage, &destination, id)
                .map(|()| None)
        } else {
            crate::vfs::promote_copy_stage_evidence(target, stage, &destination)
        };
        match promoted {
            Err(error) if replacing && replacement_refused(&error) => {
                super::file_recovery::unpublished(engine, file)
                    .map_err(|error| OpError::at(At::Unknown, error))?;
                engine.stats.replace_refused();
                engine.issues.note(
                    &file.source,
                    &format!("Ersetzen nicht möglich ({error}) – beide behalten"),
                );
                replacing = false;
            }
            Ok(published) => {
                let acknowledged = match published {
                    Some(evidence) => super::file_recovery::after_known(
                        engine,
                        file,
                        &destination,
                        Some(&super::batch_put::host_evidence(evidence)),
                    ),
                    None => super::file_recovery::after(engine, file, &destination),
                };
                acknowledged.map_err(|error| OpError::at(At::Unknown, error))?;
                if top_level {
                    engine
                        .folders
                        .record_alias(&file.rel, base_name(&destination));
                }
                return Ok(Outcome::Done);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                super::file_recovery::unpublished(engine, file).map_err(|error| OpError::at(At::Unknown, error))?;
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
                super::file_recovery::unpublished(engine, file).map_err(|error| OpError::at(At::Unknown, error))?;
                discard(engine, target, stage);
                return Err(OpError::at(At::Publish, error));
            }
            // Refused outright (full, read-only, not ours to write): nothing
            // was published, and the job ends.
            Err(error) if ends_job_at_target(error.kind()) => {
                super::file_recovery::unpublished(engine, file).map_err(|error| OpError::at(At::Unknown, error))?;
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

/// A replacement the target cannot do or that found the old file changed or
/// gone: nothing was published, the stage is published beside the old file.
fn replacement_refused(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::InvalidData | io::ErrorKind::NotFound
    )
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
