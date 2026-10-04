//! Compatibility transfer entrypoints; publication always uses an exclusive stage.
use super::apply_guard::{capture, drift, revalidate, CapturedFile, ExpectedFile};
use super::apply_retry::AttemptError;
use super::paths::{join, parent_of};
use super::transfer_stream::{check, stream};
use super::types::Throttle;
use crate::vfs::{Backend, StageDurability};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

const UNIQUE_ATTEMPTS: u64 = 1000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CopyReplacePhase {
    BackingUp,
    Copying,
}

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
    let staged = super::apply_stage::stage(
        source,
        source_path,
        &source_state,
        source_expected,
        destination,
        destination_path,
        &destination_state,
        StageDurability::Now,
        throttle,
        cancel,
        |_| {},
    )
    .map_err(AttemptError::pre_commit)?;
    if destination_state.metadata.is_some() {
        if let Some((rel, versions)) = backup {
            progress(CopyReplacePhase::BackingUp);
            back_up_captured(
                destination,
                destination_path,
                rel,
                versions,
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
    check(cancel).map_err(AttemptError::pre_commit)?;
    revalidate(source, source_path, &source_state, "copy source")
        .map_err(AttemptError::pre_commit)?;
    progress(CopyReplacePhase::Copying);
    let outcome = staged
        .publish(destination_path, &destination_state, false, cancel)
        .map_err(AttemptError::commit_attempted)?;
    super::apply_stage::require_durable(outcome.durable).map_err(AttemptError::commit_attempted)?;
    Ok(outcome.bytes)
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
    copy_conflict_sibling_at(
        backend,
        source_path,
        root,
        rel,
        expected,
        throttle,
        cancel,
        &chrono::Local::now().format("%Y%m%d-%H%M%S").to_string(),
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
    let captured = capture(backend, source_path, expected, "conflict source")
        .map_err(AttemptError::pre_commit)?;
    let missing = CapturedFile { metadata: None };
    let mut staged = super::apply_stage::stage(
        backend,
        source_path,
        &captured,
        expected,
        backend,
        source_path,
        &missing,
        StageDurability::Now,
        throttle,
        cancel,
        |_| {},
    )
    .map_err(AttemptError::pre_commit)?;
    for ordinal in 0..UNIQUE_ATTEMPTS {
        check(cancel).map_err(AttemptError::pre_commit)?;
        revalidate(backend, source_path, &captured, "conflict source")
            .map_err(AttemptError::pre_commit)?;
        let candidate = conflict_candidate_for(backend, root, rel, stamp, ordinal)
            .map_err(AttemptError::pre_commit)?;
        match staged.publish_sibling(&candidate, cancel) {
            Ok(outcome) => {
                super::apply_stage::require_durable(outcome.durable)
                    .map_err(AttemptError::commit_attempted)?;
                return Ok((outcome.bytes, candidate));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                if backend
                    .try_exists(&candidate)
                    .map_err(AttemptError::commit_attempted)?
                    && backend
                        .try_exists(&staged.path)
                        .map_err(AttemptError::commit_attempted)?
                {
                    continue;
                }
                return Err(AttemptError::commit_attempted(error));
            }
        }
    }
    Err(AttemptError::pre_commit(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique conflict sibling",
    )))
}
fn conflict_candidate_for(
    backend: &dyn Backend,
    root: &str,
    rel: &str,
    stamp: &str,
    ordinal: u64,
) -> io::Result<String> {
    let original = conflict_candidate(rel, stamp, ordinal);
    let (parent, mut name) = match original.rsplit_once('/') {
        Some((parent, name)) => (Some(parent), name.to_string()),
        None => (None, original),
    };
    let limits = crate::vfs::target_limits(backend, root);
    if limits.name_issue(&name).is_some() {
        // Only the generated sibling is shortened. The original's literal
        // spelling never changes, and the stable suffix remains unique.
        let suffix = format!(" (Konflikt {stamp} {})", ordinal + 1);
        let mut stem = rel.rsplit('/').next().unwrap_or(rel).to_string();
        loop {
            name = format!("{stem}{suffix}");
            if limits.name_issue(&name).is_none() {
                break;
            }
            if stem.pop().is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidFilename,
                    "target cannot store a conflict name",
                ));
            }
        }
    }
    crate::vfs::sync_path(
        backend,
        root,
        &parent.map_or(name.clone(), |parent| format!("{parent}/{name}")),
    )
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

pub(super) fn verify_copy(destination: &dyn Backend, path: &str, expected: u64) -> io::Result<()> {
    let state = capture(destination, path, ExpectedFile::Unknown, "copy result")?;
    if state.regular("copy result")?.size != expected {
        return Err(drift("copy verification size differs"));
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
    crate::agent_proto::ValidatedRelativePath::parse(rel)?;
    let metadata = captured.regular("backup source")?;
    let not_canceled = AtomicBool::new(false);
    let cancel = cancel.unwrap_or(&not_canceled);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    for offset in 0..UNIQUE_ATTEMPTS {
        check(cancel)?;
        let destination = versions_dir
            .join(timestamp.saturating_add(offset).to_string())
            .join(rel);
        if let Some(parent) = destination.parent() {
            crate::support_dirs::ensure_private_dir(parent)?;
        }
        let mut file = match crate::support_dirs::create_private_file(&destination) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            let mut reader = crate::vfs::open_read_regular(backend, path, metadata.id.as_deref())?;
            let result = stream(
                &mut *reader,
                &mut file,
                cancel,
                None,
                expected.hash(),
                |_| {},
            )?;
            if size_is_authoritative(backend, path, metadata) && result.bytes != metadata.size {
                return Err(drift("backup source size changed while reading"));
            }
            file.flush()?;
            file.sync_all()?;
            revalidate(backend, path, captured, "backup source")?;
            super::apply_stage::require_durable(super::apply_stage::native_namespace(&destination)?)
        })();
        if let Err(error) = result {
            drop(file);
            let _ = std::fs::remove_file(&destination);
            return Err(error);
        }
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique reversible backup",
    ))
}

pub(super) fn verify_expected_content(
    backend: &dyn Backend,
    path: &str,
    captured: &CapturedFile,
    expected: ExpectedFile,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let metadata = captured.regular("file")?;
    if expected.hash() == 0 || metadata.content_md5.is_some() {
        return Ok(());
    }
    let mut reader = crate::vfs::open_read_regular(backend, path, metadata.id.as_deref())?;
    let result = stream(
        &mut *reader,
        &mut io::sink(),
        cancel,
        None,
        expected.hash(),
        |_| {},
    )?;
    if size_is_authoritative(backend, path, metadata) && result.bytes != metadata.size {
        return Err(drift("file size changed while checking planned content"));
    }
    revalidate(backend, path, captured, "planned file")
}
fn size_is_authoritative(
    backend: &dyn Backend,
    path: &str,
    metadata: &crate::vfs::VfsMeta,
) -> bool {
    backend.download_name(path, &metadata.name) == metadata.name
}

#[cfg(test)]
#[path = "apply_transfer_tests.rs"]
mod tests;
