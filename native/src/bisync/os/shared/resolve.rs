use crate::vfs::Backend;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::apply_delete::{delete_guarded_with_progress_and_guard, DeleteGuardedPhase};
use super::apply_guard::{capture, revalidate, ExpectedFile};
use super::apply_transfer::{copy_replace, copy_replace_with_progress, CopyReplacePhase};
use super::paths::join;
use super::persistence::versions_dir;
use super::types::{Conflict, Sig, Throttle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvePhase {
    Preparing,
    BackingUp,
    Copying,
    Deleting,
    ReadingSignatures,
}

/// Resolve one conflict by copying the chosen side over the other with a
/// reversible backup of the loser. This compatibility entry point captures
/// current state at call time; interactive callers should use
/// [`resolve_checked`] so changes since conflict discovery are rejected.
pub fn resolve(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    rel: &str,
    keep_a: bool,
    pair: &str,
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    let versions = versions_dir(pair);
    let path_a = join(root_a, rel);
    let path_b = join(root_b, rel);
    let throttle = Throttle::new(0);
    let cancel = AtomicBool::new(false);
    let result = if keep_a {
        copy_replace(
            a,
            &path_a,
            ExpectedFile::Unknown,
            b,
            &path_b,
            ExpectedFile::Unknown,
            Some((rel, &versions)),
            &throttle,
            &cancel,
        )
    } else {
        copy_replace(
            b,
            &path_b,
            ExpectedFile::Unknown,
            a,
            &path_a,
            ExpectedFile::Unknown,
            Some((rel, &versions)),
            &throttle,
            &cancel,
        )
    };
    result.map_err(|error| error.into_io())?;
    Ok((sig_of(a, &path_a)?, sig_of(b, &path_b)?))
}

/// Resolve an interactive conflict against the exact signatures displayed to
/// the user. All backend I/O, including backups and post-copy stats, happens in
/// the caller's worker thread. Cancellation is honored before commit and while
/// bytes are streamed; after a commit, result signatures are still collected
/// so the caller never reports a committed resolution as merely canceled.
#[allow(clippy::too_many_arguments)]
pub fn resolve_checked(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    conflict: &Conflict,
    keep_a: bool,
    pair: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(ResolvePhase),
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    if cancel.load(Ordering::Acquire) {
        return Err(interrupted());
    }
    progress(ResolvePhase::Preparing);

    let versions = versions_dir(pair);
    let path_a = join(root_a, &conflict.rel);
    let path_b = join(root_b, &conflict.rel);
    let expected_a = expected(conflict.a);
    let expected_b = expected(conflict.b);
    let throttle = Throttle::new(0);

    let (source, source_path, source_expected, destination, destination_path, destination_expected) =
        if keep_a {
            (a, &path_a, expected_a, b, &path_b, expected_b)
        } else {
            (b, &path_b, expected_b, a, &path_a, expected_a)
        };

    if source_expected_is_present(source_expected) {
        copy_replace_with_progress(
            source,
            source_path,
            source_expected,
            destination,
            destination_path,
            destination_expected,
            Some((&conflict.rel, &versions)),
            &throttle,
            cancel,
            |phase| {
                progress(match phase {
                    CopyReplacePhase::BackingUp => ResolvePhase::BackingUp,
                    CopyReplacePhase::Copying => ResolvePhase::Copying,
                })
            },
        )
        .map_err(|error| error.into_io())?;
    } else if destination_expected_is_present(destination_expected) {
        let missing_source = capture(source, source_path, source_expected, "chosen conflict side")?;
        delete_guarded_with_progress_and_guard(
            destination,
            destination_path,
            &conflict.rel,
            destination_expected,
            true,
            &versions,
            false,
            cancel,
            |phase| {
                progress(match phase {
                    DeleteGuardedPhase::BackingUp => ResolvePhase::BackingUp,
                    DeleteGuardedPhase::Deleting => ResolvePhase::Deleting,
                })
            },
            || revalidate(source, source_path, &missing_source, "chosen conflict side"),
        )
        .map_err(|error| error.into_io())?;
    } else {
        // Both displayed sides are absent. Revalidate that fact before treating
        // the conflict as converged; no filesystem mutation is necessary.
        capture(a, &path_a, expected_a, "conflict side A")?;
        capture(b, &path_b, expected_b, "conflict side B")?;
    }

    // Do not honor a late cancel after a mutation may have committed: collect
    // authoritative result state so the UI can update its baseline correctly.
    progress(ResolvePhase::ReadingSignatures);
    let signatures = (sig_of(a, &path_a)?, sig_of(b, &path_b)?);
    if !source_expected_is_present(source_expected)
        && (signatures.0.is_some() || signatures.1.is_some())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chosen deleted side changed while resolving the conflict",
        ));
    }
    Ok(signatures)
}

fn expected(signature: Option<Sig>) -> ExpectedFile {
    signature.map_or(ExpectedFile::Missing, ExpectedFile::Present)
}

fn source_expected_is_present(expected: ExpectedFile) -> bool {
    matches!(expected, ExpectedFile::Present(_))
}

fn destination_expected_is_present(expected: ExpectedFile) -> bool {
    source_expected_is_present(expected)
}

fn sig_of(backend: &dyn Backend, path: &str) -> io::Result<Option<Sig>> {
    match backend.stat(path) {
        Ok(metadata) if metadata.is_dir || metadata.is_symlink => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("resolved conflict is not a regular file: {path}"),
        )),
        Ok(metadata) => Ok(Some(Sig {
            size: metadata.size,
            mtime_ms: metadata.mtime_ms,
            hash: 0,
        })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "conflict resolution canceled")
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
