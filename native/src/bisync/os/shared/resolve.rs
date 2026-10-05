use crate::vfs::{self, Backend};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::apply_delete::{delete_guarded_with_progress_and_guard, DeleteGuardedPhase};
use super::apply_guard::{capture, revalidate, ExpectedFile};
use super::apply_transfer::{copy_replace, copy_replace_with_progress, CopyReplacePhase};
use super::pair_lock::PairLock;
use super::persistence::versions_dir;
use super::replica_state::merge_baseline_entries;
use super::run_types::StateKey;
use super::types::{Action, BisyncOptions, Conflict, PairSide, Sig, Throttle};

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
    super::apply_boundary::target(a, root_a, rel, None)?;
    super::apply_boundary::target(b, root_b, rel, None)?;
    let versions = versions_dir(pair);
    let path_a = vfs::sync_path(a, root_a, rel)?;
    let path_b = vfs::sync_path(b, root_b, rel)?;
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
    progress: impl FnMut(ResolvePhase),
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    resolve_variant_checked(
        a, root_a, b, root_b, conflict, keep_a, None, pair, cancel, progress,
    )
}

/// Resolves one conflict under the pair's lock and records the result in the
/// stored state the conflict came from (V3, Y45): no run works on the pair
/// meanwhile and nothing a run recorded before is overwritten.
/// `ErrorKind::WouldBlock` while a run of the pair is in progress.
#[allow(clippy::too_many_arguments)]
pub fn resolve_recorded(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    conflict: &Conflict,
    keep_a: bool,
    variant_id: Option<&str>,
    state: &StateKey,
    cancel: &AtomicBool,
    progress: impl FnMut(ResolvePhase),
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    let lock = PairLock::acquire(&state.lock_id)?;
    let endpoints = super::incremental::SyncEndpoints::new(a, root_a, b, root_b);
    super::single_recorded::validate_state(endpoints, state)?;
    let keys = super::orchestration_plan::keys(endpoints);
    if super::merge_resume::pending_merge_relatives(&lock, state)?
        .iter()
        .any(|rel| keys.key(rel) == keys.key(&conflict.rel))
    {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Dieser Pfad hat eine unbeendete Merge-Auflösung; denselben Merge erneut bestätigen",
        ));
    }
    if conflict.duplicates.is_none() {
        if variant_id.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Dieser Konflikt hat keine auswählbare Datei-ID",
            ));
        }
        return resolve_single_recorded(
            endpoints, &lock, conflict, keep_a, state, cancel, progress,
        );
    }
    let signatures = resolve_variant_checked(
        a,
        root_a,
        b,
        root_b,
        conflict,
        keep_a,
        variant_id,
        &state.pair_id,
        cancel,
        progress,
    )?;
    super::replica_state::merge_with_keys(
        &lock,
        state,
        &[(conflict.rel.clone(), signatures)],
        super::orchestration_plan::keys(endpoints),
    )?;
    Ok(signatures)
}

#[allow(clippy::too_many_arguments)]
fn resolve_single_recorded(
    endpoints: super::incremental::SyncEndpoints<'_>,
    lock: &PairLock,
    conflict: &Conflict,
    keep_a: bool,
    state: &StateKey,
    cancel: &AtomicBool,
    mut progress: impl FnMut(ResolvePhase),
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    if cancel.load(Ordering::Acquire) {
        return Err(interrupted());
    }
    progress(ResolvePhase::Preparing);
    let keys = super::orchestration_plan::keys(endpoints);
    let names = super::state_spellings::load(state, keys)?;
    let mut spellings = super::Spellings::default();
    for side in [PairSide::A, PairSide::B] {
        let rel = names.rel(&conflict.rel, side, keys);
        let (backend, root) = if side == PairSide::A {
            (endpoints.a, endpoints.root_a)
        } else {
            (endpoints.b, endpoints.root_b)
        };
        super::apply_boundary::target(backend, root, &rel, None)?;
        spellings.insert(&conflict.rel, side, &rel);
    }
    let action = match (keep_a, conflict.a, conflict.b) {
        (true, Some(_), _) => Action::CopyAtoB(conflict.rel.clone()),
        (false, _, Some(_)) => Action::CopyBtoA(conflict.rel.clone()),
        (true, None, Some(_)) => Action::DeleteB(conflict.rel.clone()),
        (false, Some(_), None) => Action::DeleteA(conflict.rel.clone()),
        (_, None, None) => {
            capture(
                endpoints.a,
                &vfs::sync_path(
                    endpoints.a,
                    endpoints.root_a,
                    spellings.side_rel(&conflict.rel, PairSide::A),
                )?,
                ExpectedFile::Missing,
                "conflict side A",
            )?;
            capture(
                endpoints.b,
                &vfs::sync_path(
                    endpoints.b,
                    endpoints.root_b,
                    spellings.side_rel(&conflict.rel, PairSide::B),
                )?,
                ExpectedFile::Missing,
                "conflict side B",
            )?;
            super::replica_state::merge_with_keys(
                lock,
                state,
                &[(conflict.rel.clone(), (None, None))],
                keys,
            )?;
            return Ok((None, None));
        }
    };
    progress(ResolvePhase::BackingUp);
    progress(
        if matches!(action, Action::DeleteA(_) | Action::DeleteB(_)) {
            ResolvePhase::Deleting
        } else {
            ResolvePhase::Copying
        },
    );
    let mut opts = match &state.owner {
        super::StateOwner::Job(id) => crate::syncjobs::recorded_options(id)?,
        super::StateOwner::AdHoc => BisyncOptions::default(),
    };
    // Choosing one conflict side copies that file; the job's later move
    // phase remains a separate guarded action, as in resolve_checked.
    opts.direction = super::Direction::Both;
    opts.move_files = false;
    opts.reversible = true;
    let (_, entry) = super::single_recorded::apply_one(
        endpoints,
        lock,
        state,
        &action,
        (conflict.a, conflict.b),
        &spellings,
        opts,
        cancel,
    )?;
    progress(ResolvePhase::ReadingSignatures);
    Ok(entry)
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_variant_checked(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    conflict: &Conflict,
    keep_a: bool,
    variant_id: Option<&str>,
    pair: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(ResolvePhase),
) -> io::Result<(Option<Sig>, Option<Sig>)> {
    if cancel.load(Ordering::Acquire) {
        return Err(interrupted());
    }
    progress(ResolvePhase::Preparing);

    super::apply_boundary::target(a, root_a, &conflict.rel, None)?;
    super::apply_boundary::target(b, root_b, &conflict.rel, None)?;

    if conflict.duplicates.is_some() {
        return super::duplicate_apply::resolve(
            super::incremental::SyncEndpoints::new(a, root_a, b, root_b),
            conflict,
            keep_a,
            variant_id,
            &versions_dir(pair),
            cancel,
            0,
            progress,
        );
    }
    if variant_id.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Dieser Konflikt hat keine auswählbare Datei-ID",
        ));
    }

    let versions = versions_dir(pair);
    let path_a = vfs::sync_path(a, root_a, &conflict.rel)?;
    let path_b = vfs::sync_path(b, root_b, &conflict.rel)?;
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
    match vfs::sync_stat(backend, path) {
        Ok(metadata) if metadata.is_dir || metadata.is_symlink || metadata.special => {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("resolved conflict is not a regular file: {path}"),
            ))
        }
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
