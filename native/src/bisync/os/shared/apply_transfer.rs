use crate::vfs::Backend;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::apply_guard::{capture, drift, revalidate, CapturedFile, ExpectedFile};
use super::apply_retry::AttemptError;
use super::paths::{join, parent_of};
use super::snapshot_hash::md5_to_u64;
use super::types::Throttle;

const COPY_BUFFER: usize = 256 * 1024;
const UNIQUE_ATTEMPTS: u64 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CopyReplacePhase {
    BackingUp,
    Copying,
}

// This transaction boundary intentionally spells out both endpoint states and
// its backup, throttle, and cancellation policy so preconditions cannot mix.
#[allow(clippy::too_many_arguments)]
pub(super) fn copy_replace(
    source: &dyn Backend,
    source_path: &str,
    source_expected: ExpectedFile,
    destination: &dyn Backend,
    destination_path: &str,
    destination_expected: ExpectedFile,
    backup: Option<(&str, &Path)>,
    throttle: &Throttle,
    cancel: &AtomicBool,
) -> Result<u64, AttemptError> {
    copy_replace_with_progress(
        source,
        source_path,
        source_expected,
        destination,
        destination_path,
        destination_expected,
        backup,
        throttle,
        cancel,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn copy_replace_with_progress(
    source: &dyn Backend,
    source_path: &str,
    source_expected: ExpectedFile,
    destination: &dyn Backend,
    destination_path: &str,
    destination_expected: ExpectedFile,
    backup: Option<(&str, &Path)>,
    throttle: &Throttle,
    cancel: &AtomicBool,
    mut progress: impl FnMut(CopyReplacePhase),
) -> Result<u64, AttemptError> {
    let source_state = capture(source, source_path, source_expected, "copy source")
        .map_err(AttemptError::pre_commit)?;
    source_state
        .regular("copy source")
        .map_err(AttemptError::pre_commit)?;
    let destination_state = capture(
        destination,
        destination_path,
        destination_expected,
        "copy destination",
    )
    .map_err(AttemptError::pre_commit)?;

    if destination_state.metadata.is_some() {
        if let Some((rel, versions_dir)) = backup {
            progress(CopyReplacePhase::BackingUp);
            back_up_captured(
                destination,
                destination_path,
                rel,
                versions_dir,
                &destination_state,
                destination_expected,
                Some(cancel),
            )
            .map_err(AttemptError::pre_commit)?;
        } else {
            verify_expected_content(
                destination,
                destination_path,
                &destination_state,
                destination_expected,
                cancel,
            )
            .map_err(AttemptError::pre_commit)?;
        }
    }
    revalidate(
        destination,
        destination_path,
        &destination_state,
        "copy destination",
    )
    .map_err(AttemptError::pre_commit)?;

    progress(CopyReplacePhase::Copying);
    let (staged, copied) = stage_source(
        source,
        source_path,
        &source_state,
        source_expected,
        destination,
        destination_path,
        throttle,
        cancel,
    )
    .map_err(AttemptError::pre_commit)?;
    if let Err(error) = revalidate(
        destination,
        destination_path,
        &destination_state,
        "copy destination",
    ) {
        let _ = destination.remove_file(&staged);
        return Err(AttemptError::pre_commit(error));
    }
    if cancel.load(Ordering::Relaxed) {
        let _ = destination.remove_file(&staged);
        return Err(AttemptError::pre_commit(interrupted()));
    }
    let promote_result = if destination_state.metadata.is_some() {
        crate::vfs::promote_staged_replace(destination, &staged, destination_path)
    } else {
        crate::vfs::promote_staged_create(destination, &staged, destination_path)
    };
    if let Err(error) = promote_result {
        let _ = destination.remove_file(&staged);
        return Err(AttemptError::commit_attempted(error));
    }
    Ok(copied)
}

pub(super) fn copy_conflict_sibling(
    backend: &dyn Backend,
    source_path: &str,
    root: &str,
    rel: &str,
    expected: ExpectedFile,
    throttle: &Throttle,
    cancel: &AtomicBool,
) -> Result<(u64, String), AttemptError> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    copy_conflict_sibling_at(
        backend,
        source_path,
        root,
        rel,
        expected,
        throttle,
        cancel,
        &stamp,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn copy_conflict_sibling_at(
    backend: &dyn Backend,
    source_path: &str,
    root: &str,
    rel: &str,
    expected: ExpectedFile,
    throttle: &Throttle,
    cancel: &AtomicBool,
    stamp: &str,
) -> Result<(u64, String), AttemptError> {
    let source_state = capture(backend, source_path, expected, "conflict source")
        .map_err(AttemptError::pre_commit)?;
    source_state
        .regular("conflict source")
        .map_err(AttemptError::pre_commit)?;
    let (staged, copied) = stage_source(
        backend,
        source_path,
        &source_state,
        expected,
        backend,
        source_path,
        throttle,
        cancel,
    )
    .map_err(AttemptError::pre_commit)?;

    for ordinal in 0..UNIQUE_ATTEMPTS {
        if cancel.load(Ordering::Relaxed) {
            let _ = backend.remove_file(&staged);
            return Err(AttemptError::pre_commit(interrupted()));
        }
        let candidate = join(root, &conflict_candidate(rel, stamp, ordinal));
        match backend.rename_no_replace(&staged, &candidate) {
            Ok(()) => {
                let metadata = capture(backend, &candidate, ExpectedFile::Unknown, "conflict copy")
                    .and_then(|state| state.regular("conflict copy").cloned())
                    .map_err(AttemptError::commit_attempted)?;
                if metadata.size != copied {
                    return Err(AttemptError::commit_attempted(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "published conflict copy has the wrong size",
                    )));
                }
                return Ok((copied, candidate));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => match failed_no_replace_is_collision(backend, &staged, &candidate) {
                Ok(true) => continue,
                Ok(false) => {
                    let _ = backend.remove_file(&staged);
                    return Err(AttemptError::commit_attempted(error));
                }
                Err(probe_error) => {
                    return Err(AttemptError::commit_attempted(io::Error::new(
                        probe_error.kind(),
                        format!(
                            "conflict rename failed ({error}); outcome probe failed: {probe_error}"
                        ),
                    )));
                }
            },
        }
    }
    let _ = backend.remove_file(&staged);
    Err(AttemptError::pre_commit(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique conflict sibling",
    )))
}

pub(super) fn verify_copy(destination: &dyn Backend, path: &str, expected: u64) -> io::Result<()> {
    let state = capture(destination, path, ExpectedFile::Unknown, "copy result")?;
    let got = state.regular("copy result")?.size;
    if got != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("copy verification failed: {got} != {expected} bytes"),
        ));
    }
    Ok(())
}

pub(super) fn back_up(
    backend: &dyn Backend,
    path: &str,
    rel: &str,
    versions_dir: &Path,
) -> io::Result<()> {
    let state = capture(backend, path, ExpectedFile::Unknown, "backup source")?;
    state.regular("backup source")?;
    back_up_captured(
        backend,
        path,
        rel,
        versions_dir,
        &state,
        ExpectedFile::Unknown,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn back_up_captured(
    backend: &dyn Backend,
    path: &str,
    rel: &str,
    versions_dir: &Path,
    captured: &CapturedFile,
    expected: ExpectedFile,
    cancel: Option<&AtomicBool>,
) -> io::Result<()> {
    let metadata = captured.regular("backup source")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    for offset in 0..UNIQUE_ATTEMPTS {
        let destination = versions_dir
            .join(timestamp.saturating_add(offset).to_string())
            .join(rel);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            let mut reader = backend.open_read_id(path, metadata.id.as_deref())?;
            let streamed = stream(&mut *reader, &mut file, cancel, None, expected.hash())?;
            if size_is_authoritative(backend, path, metadata) && streamed.bytes != metadata.size {
                return Err(drift("backup source size changed while reading"));
            }
            file.flush()?;
            file.sync_all()?;
            revalidate(backend, path, captured, "backup source")
        })();
        if let Err(error) = result {
            drop(file);
            let _ = std::fs::remove_file(destination);
            return Err(error);
        }
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique reversible backup",
    ))
}

// Staging has distinct captured-state and planned-state inputs for the source,
// plus destination and lifecycle controls; keeping each explicit aids auditing.
#[allow(clippy::too_many_arguments)]
fn stage_source(
    source: &dyn Backend,
    source_path: &str,
    source_state: &CapturedFile,
    source_expected: ExpectedFile,
    destination: &dyn Backend,
    destination_path: &str,
    throttle: &Throttle,
    cancel: &AtomicBool,
) -> io::Result<(String, u64)> {
    let metadata = source_state.regular("copy source")?;
    if let Some(parent) = parent_of(destination_path) {
        destination.mkdir_all(&parent)?;
    }
    let staged = crate::vfs::unique_staging_path(destination, destination_path, "bisync")?;
    let result = (|| {
        let mut reader = source.open_read_id(source_path, metadata.id.as_deref())?;
        let mut writer = destination.open_write(&staged)?;
        let streamed = stream(
            &mut *reader,
            &mut *writer,
            Some(cancel),
            Some(throttle),
            source_expected.hash(),
        )?;
        writer.flush()?;
        drop(writer);
        if size_is_authoritative(source, source_path, metadata) && streamed.bytes != metadata.size {
            return Err(drift("copy source size changed while reading"));
        }
        revalidate(source, source_path, source_state, "copy source")?;
        let staged_state = capture(destination, &staged, ExpectedFile::Unknown, "staged copy")?;
        if staged_state.regular("staged copy")?.size != streamed.bytes {
            return Err(drift("staged copy has the wrong size"));
        }
        Ok(streamed.bytes)
    })();
    match result {
        Ok(bytes) => Ok((staged, bytes)),
        Err(error) => {
            let _ = destination.remove_file(&staged);
            Err(error)
        }
    }
}

struct Streamed {
    bytes: u64,
}

fn stream(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    cancel: Option<&AtomicBool>,
    throttle: Option<&Throttle>,
    expected_hash: u64,
) -> io::Result<Streamed> {
    let mut context = (expected_hash != 0).then(md5::Context::new);
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut bytes = 0u64;
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(interrupted());
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        if let Some(context) = context.as_mut() {
            context.consume(&buffer[..read]);
        }
        bytes = bytes
            .checked_add(read as u64)
            .ok_or_else(|| drift("file size overflow while copying"))?;
        if let Some(throttle) = throttle {
            throttle.consume(read as u64);
        }
    }
    if let Some(context) = context {
        let actual = md5_to_u64(&context.compute().0);
        if actual != expected_hash {
            return Err(drift("file content changed since planning"));
        }
    }
    Ok(Streamed { bytes })
}

pub(super) fn verify_expected_content(
    backend: &dyn Backend,
    path: &str,
    captured: &CapturedFile,
    expected: ExpectedFile,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let expected_hash = expected.hash();
    if expected_hash == 0 || captured.regular("file")?.content_md5.is_some() {
        return Ok(());
    }
    let metadata = captured.regular("file")?;
    let mut reader = backend.open_read_id(path, metadata.id.as_deref())?;
    let streamed = stream(
        &mut *reader,
        &mut io::sink(),
        Some(cancel),
        None,
        expected_hash,
    )?;
    if size_is_authoritative(backend, path, metadata) && streamed.bytes != metadata.size {
        return Err(drift("file size changed while checking planned content"));
    }
    revalidate(backend, path, captured, "planned file")
}

fn conflict_candidate(rel: &str, stamp: &str, ordinal: u64) -> String {
    let suffix = if ordinal == 0 {
        format!(" (Konflikt {stamp})")
    } else {
        format!(" (Konflikt {stamp} {})", ordinal + 1)
    };
    match rel.rfind('.') {
        Some(index) if index > rel.rfind('/').map(|slash| slash + 1).unwrap_or(0) => {
            format!("{}{}{}", &rel[..index], suffix, &rel[index..])
        }
        _ => format!("{rel}{suffix}"),
    }
}

fn failed_no_replace_is_collision(
    backend: &dyn Backend,
    staged: &str,
    candidate: &str,
) -> io::Result<bool> {
    // Some protocols (notably SFTP v3) report an undifferentiated Failure when
    // the atomic rename refused an existing destination. It is a safe name
    // collision only when the candidate exists AND our stage still exists.
    Ok(backend.try_exists(candidate)? && backend.try_exists(staged)?)
}

fn size_is_authoritative(
    backend: &dyn Backend,
    path: &str,
    metadata: &crate::vfs::VfsMeta,
) -> bool {
    backend.download_name(path, &metadata.name) == metadata.name
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "synchronization copy canceled")
}

#[cfg(test)]
#[path = "apply_transfer_tests.rs"]
mod tests;
