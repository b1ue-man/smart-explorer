//! The private stage of one local copy: the source is opened and observed,
//! its bytes go into an exclusively created, unpredictable sibling of the
//! destination through the kernel (K12), and the source must be unchanged
//! afterwards. New copies are not synced to disk file by file (like Explorer
//! and cp); moves and replacements are.
use std::collections::hash_map::RandomState;
use std::fs::File;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::platform;
use super::move_guard::{
    file_state, source_changed_error, source_snapshot_entry, source_snapshot_file, SourceSnapshot,
};

/// Unpredictable stage names collide only with a foreign file of that exact
/// name; the bound is caution, as in the older copy code.
const STAGE_ATTEMPTS: u32 = 1000;

/// Which side of a local copy failed.
#[derive(Debug)]
pub(crate) enum LocalFailure {
    /// Reading the source (nothing was published).
    Source(io::Error),
    /// Creating or writing the stage (nothing was published).
    Target(io::Error),
    /// Publishing (including examining the one entry it would replace) or
    /// finishing a published copy: this file's failure, never the target's.
    Publish(io::Error),
}

/// Lets platform copy code that knows which side failed carry the verdict
/// inside an `io::Error` (`io::Error::other(LocalFailure::Target(..))`): the
/// unbuffered Linux copy reads and writes itself, so a read error is the
/// source's and a write error the target's (`side_of` unwraps it).
impl std::fmt::Display for LocalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalFailure::Source(error) => write!(f, "source: {error}"),
            LocalFailure::Target(error) => write!(f, "target: {error}"),
            LocalFailure::Publish(error) => write!(f, "publish: {error}"),
        }
    }
}

impl std::error::Error for LocalFailure {}

impl LocalFailure {
    pub(crate) fn into_error(self) -> io::Error {
        match self {
            LocalFailure::Source(error)
            | LocalFailure::Target(error)
            | LocalFailure::Publish(error) => error,
        }
    }
}

/// A complete, private copy of the source.
pub(super) struct Staged {
    pub(super) temp: PathBuf,
    pub(super) file: File,
    pub(super) bytes: u64,
    pub(super) snapshot: SourceSnapshot,
}

impl Staged {
    pub(super) fn discard(self) -> io::Result<()> {
        discard(self.temp, self.file)
    }
}

/// How many threads may copy ranges of one file at once, on platforms that
/// can (`copy_handles`), and whether the drive is measured first.
pub(super) struct Parallel<'a> {
    pub threads: usize,
    /// Set for a drive not known to be flash: the copy times one section with
    /// one thread and one with two and calls this with whether two helped; the
    /// rest then uses all `threads` or one (`copy_probed`; Windows has no
    /// ranges and copies plainly).
    pub probe: Option<&'a dyn Fn(bool)>,
}

/// Copies `source` into a new stage next to `target`; `None` when canceled.
/// `durable` (moves, replacements) syncs the stage to disk. `parallel` is
/// what the copy may use; durable copies get one thread and no measurement.
pub(super) fn stage_copy(
    source: &Path,
    target: &Path,
    cancel: &AtomicBool,
    durable: bool,
    preserve_destination_mode: bool,
    parallel: &Parallel<'_>,
    progress: &mut dyn FnMut(u64),
) -> Result<Option<Staged>, LocalFailure> {
    // `transfer_local` looked at the path a moment ago; the open below checks
    // the type again and the final comparison catches any swap. A regular
    // file's medium is its folder's (`acquire_regular_file`, M6).
    let optical = match crate::source_media::acquire_regular_file(
        source,
        crate::source_media::Priority::Foreground,
        cancel,
    ) {
        Ok(guard) => guard,
        Err(_error) if cancel.load(std::sync::atomic::Ordering::Acquire) => return Ok(None),
        Err(error) => return Err(LocalFailure::Source(error)),
    };
    if let Some(guard) = &optical {
        guard.set_mode(crate::source_media::ReadMode::Sequential);
    }
    let single = Parallel {
        threads: 1,
        probe: None,
    };
    let parallel = if optical.is_some() { &single } else { parallel };
    let (reader, before) = open_regular_source(source)?;
    if !durable {
        if let Some(staged) = stage_by_path(
            source,
            target,
            &reader,
            &before,
            cancel,
            optical.is_some(),
            progress,
        )? {
            if let Some(guard) = &optical {
                if let Err(error) = guard.validate(source) {
                    if let Some(staged) = staged {
                        staged.discard().map_err(LocalFailure::Target)?;
                    }
                    return Err(LocalFailure::Source(error));
                }
            }
            return Ok(staged);
        }
    }
    let (temp, mut writer) = create_temp_sibling(target).map_err(LocalFailure::Target)?;
    let copied = match if optical.is_some() {
        copy_optical_handles(&reader, &mut writer, cancel, progress)
    } else {
        copy_into(&reader, &mut writer, cancel, progress, parallel)
    } {
        Ok(Some(copied)) => copied,
        Ok(None) => {
            discard(temp, writer).map_err(LocalFailure::Target)?;
            return Ok(None);
        }
        Err(error) => {
            return Err(discard_after_error(temp, writer, side_of(error)));
        }
    };
    let finished = optical
        .as_ref()
        .map_or(Ok(()), |guard| guard.validate(source))
        .map_err(LocalFailure::Source)
        .and_then(|()| unchanged_state(source, &reader, &before))
        .and_then(|metadata| {
            platform::copy_permissions(
                &writer,
                &metadata,
                preserve_destination_mode.then_some(target),
            )
            .map_err(LocalFailure::Target)
        })
        .and_then(|()| {
            if durable {
                writer.sync_all().map_err(LocalFailure::Target)
            } else {
                Ok(())
            }
        });
    if let Err(failure) = finished {
        return Err(discard_after_error(temp, writer, failure));
    }
    Ok(Some(Staged {
        temp,
        file: writer,
        bytes: copied,
        snapshot: before,
    }))
}

/// The bytes of the source into the stage: measured ranges on a drive that is
/// not known to be flash (platforms with ranges only), else `copy_handles`.
fn copy_into(
    reader: &File,
    writer: &mut File,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    parallel: &Parallel<'_>,
) -> io::Result<Option<u64>> {
    if let (Some(report), true) = (parallel.probe, parallel.threads > 1) {
        return platform::copy_probed(reader, writer, cancel, progress, parallel.threads, report);
    }
    platform::copy_handles(reader, writer, cancel, progress, parallel.threads)
}

/// Optical bytes use one bounded sequential loop on both desktop systems.
/// Preserve the existing source/target error attribution while sharing the
/// measured block parameter with Windows' metadata-preserving path.
pub(super) fn copy_optical_handles(
    reader: &File,
    writer: &mut File,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<u64>> {
    struct SourceReader<'a>(&'a File);
    impl io::Read for SourceReader<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            io::Read::read(&mut self.0, buffer).map_err(|error| {
                if error.kind() == io::ErrorKind::Interrupted {
                    error
                } else {
                    io::Error::other(LocalFailure::Source(error))
                }
            })
        }
    }
    struct TargetWriter<'a>(&'a mut File);
    impl io::Write for TargetWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            io::Write::write(self.0, bytes).map_err(|error| {
                if error.kind() == io::ErrorKind::Interrupted {
                    error
                } else {
                    io::Error::other(LocalFailure::Target(error))
                }
            })
        }
        fn flush(&mut self) -> io::Result<()> {
            io::Write::flush(self.0).map_err(|error| io::Error::other(LocalFailure::Target(error)))
        }
    }
    crate::source_media::sequential_copy(
        &mut SourceReader(reader),
        &mut TargetWriter(writer),
        cancel,
        progress,
    )
}

/// Opens the source of a copy: a regular file (never a link, junction or
/// special file), observed so a change during the copy is noticed.
pub(super) fn open_source(source: &Path) -> Result<(File, SourceSnapshot), LocalFailure> {
    let link_metadata =
        crate::local_access::symlink_metadata(source).map_err(LocalFailure::Source)?;
    if crate::local_access::metadata_is_link_like(source, &link_metadata)
        || !link_metadata.is_file()
    {
        return Err(LocalFailure::Source(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{}: keine reguläre Datei (Links, Reparse-Punkte und Spezialdateien werden nicht übertragen)",
                source.display()
            ),
        )));
    }
    open_regular_source(source)
}

/// Opens the regular file `source` without following a final link and
/// without waiting on a FIFO that replaced the file (`open_regular` checks
/// the type before and after opening) and takes its snapshot from the open
/// handle. That the path still names this file is compared once, after the
/// copy (`unchanged_state`): a swap in between cannot reach a publication.
pub(super) fn open_regular_source(source: &Path) -> Result<(File, SourceSnapshot), LocalFailure> {
    let reader = crate::local_access::open_regular(source, crate::local_access::FinalLink::Refuse)
        .map_err(LocalFailure::Source)?;
    let before = source_snapshot_file(&reader).map_err(LocalFailure::Source)?;
    Ok((reader, before))
}

/// Platforms that copy by path in the kernel (Windows `CopyFile2`) do it
/// onto a fresh name that must not exist; `Ok(None)` = use the handles.
fn stage_by_path(
    source: &Path,
    target: &Path,
    reader: &File,
    before: &SourceSnapshot,
    cancel: &AtomicBool,
    optical: bool,
    progress: &mut dyn FnMut(u64),
) -> Result<Option<Option<Staged>>, LocalFailure> {
    for attempt in 0..STAGE_ATTEMPTS {
        let temp = stage_name(target, attempt);
        let known = crate::copy::SourceFacts {
            size: before.len,
            optical,
        };
        match platform::copy_by_path(source, &temp, cancel, progress, Some(known)) {
            None => return Ok(None),
            Some(Ok(None)) => return Ok(Some(None)),
            Some(Ok(Some((bytes, file)))) => {
                if bytes != before.len || unchanged(source, reader, before).is_err() {
                    return Err(discard_after_error(
                        temp,
                        file,
                        LocalFailure::Source(source_changed_error(source)),
                    ));
                }
                return Ok(Some(Some(Staged {
                    temp,
                    file,
                    bytes,
                    snapshot: before.clone(),
                })));
            }
            Some(Err(error)) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Some(Err(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
                ) =>
            {
                return Err(LocalFailure::Target(error));
            }
            Some(Err(error)) if optical => return Err(side_of(error)),
            // Protected sources (readable only through `local_access`) and
            // file systems without the kernel path use the handles.
            Some(Err(_)) => return Ok(None),
        }
    }
    Ok(None)
}

pub(super) fn unchanged(
    source: &Path,
    reader: &File,
    before: &SourceSnapshot,
) -> Result<(), LocalFailure> {
    unchanged_state(source, reader, before).map(|_| ())
}

/// The source is still the file that was opened, with the same identity,
/// length and modification time, both through the handle and at its path.
/// Returns the handle's metadata read for the check (the permissions to copy).
pub(super) fn unchanged_state(
    source: &Path,
    reader: &File,
    before: &SourceSnapshot,
) -> Result<std::fs::Metadata, LocalFailure> {
    let (after, metadata) = file_state(reader).map_err(LocalFailure::Source)?;
    if after != *before || source_snapshot_entry(source).map_err(LocalFailure::Source)? != *before {
        return Err(LocalFailure::Source(source_changed_error(source)));
    }
    Ok(metadata)
}

/// Errors of a combined read/write copy: the ones only a destination can
/// cause belong to the target (a full or read-only volume ends the job), the
/// rest to the source (a refused read may still be granted).
fn side_of(error: io::Error) -> LocalFailure {
    if error
        .get_ref()
        .is_some_and(|inner| inner.is::<LocalFailure>())
    {
        return untagged(error);
    }
    match error.kind() {
        io::ErrorKind::StorageFull
        | io::ErrorKind::QuotaExceeded
        | io::ErrorKind::ReadOnlyFilesystem
        | io::ErrorKind::FileTooLarge
        | io::ErrorKind::WriteZero => LocalFailure::Target(error),
        _ => LocalFailure::Source(error),
    }
}

/// The failure a platform copy tagged with its side (see `side_of`).
fn untagged(error: io::Error) -> LocalFailure {
    match error.into_inner() {
        Some(inner) => match inner.downcast::<LocalFailure>() {
            Ok(failure) => *failure,
            Err(inner) => LocalFailure::Source(io::Error::other(inner)),
        },
        None => LocalFailure::Source(io::Error::other("copy failed")),
    }
}

fn discard(temp: PathBuf, file: File) -> io::Result<()> {
    let result = (|| {
        let parent_path = temp
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = crate::local_access::ConfinedDirectory::open_root(parent_path)?;
        let name = temp
            .file_name()
            .ok_or_else(|| io::Error::other("stage has no child name"))?;
        let expected = match parent.entry(name) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let class = crate::local_access::classify_open_file(&expected)?;
        if !expected.metadata()?.is_file()
            || class.link_like
            || class.special
            || !platform::same_file_object(&expected, &file)?
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "owned stage name was replaced",
            ));
        }
        // Keep the metadata/O_PATH handle alive while releasing the writer.
        // Windows's conditional DELETE open must not conflict with our own
        // write access. The object cannot be freed/reused during this handoff.
        drop(file);
        super::recovery::discard_transfer_stage_live(&parent, &temp, &expected)
    })();
    result.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "stage cleanup unconfirmed for recorded path {}: {error}",
                temp.display(),
            ),
        )
    })
}

fn discard_after_error(temp: PathBuf, file: File, cause: LocalFailure) -> LocalFailure {
    match discard(temp, file) {
        Ok(()) => cause,
        Err(error) => {
            LocalFailure::Target(io::Error::new(error.kind(), format!("{cause}; {error}")))
        }
    }
}

fn stage_name(target: &Path, attempt: u32) -> PathBuf {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "copy".to_string());
    let tail = format!(
        ".smart-explorer-{:016x}.part",
        random_suffix(target, attempt)
    );
    parent.join(crate::vfs::fit_stage_name(".", &name, &tail))
}

pub(super) fn create_temp_sibling(target: &Path) -> io::Result<(PathBuf, File)> {
    for attempt in 0..STAGE_ATTEMPTS {
        let candidate = stage_name(target, attempt);
        match crate::vfs::create_local_copy_stage(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique staged-copy name",
    ))
}

fn random_suffix(target: &Path, attempt: u32) -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    target.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    attempt.hash(&mut hasher);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
#[path = "staging_tests.rs"]
mod tests;
