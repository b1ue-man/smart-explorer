//! One guarded copy or deletion; backups must finish before mutation.
use super::apply_guard::{capture, revalidate, CapturedFile, ExpectedFile};
pub(crate) use super::apply_mirror::{quick_copy, quick_delete};
use super::apply_retry::AttemptError;
use super::apply_stage::CopyOutcome;
use super::completion::ApplySink;
use super::transfer_stream::check;
use super::types::{BisyncOptions, Throttle};
use super::version_save::{self, Preserved};
use super::versions::{RunVersions, VersionReason, VersionSide};
use crate::vfs::{Backend, StageDurability};
use std::io;
use std::sync::atomic::AtomicBool;

pub(super) fn gate(cancel: &AtomicBool, sink: Option<&dyn ApplySink>) -> io::Result<()> {
    check(cancel)?;
    if sink.is_some_and(|sink| sink.should_stop()) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "checkpoint stopped admission",
        ))
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn copy(
    source: &dyn Backend,
    source_path: &str,
    source_expected: ExpectedFile,
    destination: &VersionSide<'_>,
    destination_path: &str,
    destination_expected: ExpectedFile,
    rel: &str,
    opts: BisyncOptions,
    versions: Option<&RunVersions>,
    durability: StageDurability,
    throttle: &Throttle,
    cancel: &AtomicBool,
    sink: Option<&dyn ApplySink>,
    mut progress: impl FnMut(super::apply_transfer::CopyReplacePhase),
    bytes: impl FnMut(u64),
) -> Result<CopyOutcome, AttemptError> {
    gate(cancel, sink).map_err(AttemptError::pre_commit)?;
    let source_state = capture(source, source_path, source_expected, "copy source")
        .map_err(AttemptError::pre_commit)?;
    source_state
        .regular("copy source")
        .map_err(AttemptError::pre_commit)?;
    super::apply_boundary::guard(
        destination.backend,
        destination.root,
        rel,
        opts.cross_mounts,
    )
    .map_err(AttemptError::pre_commit)?;
    let destination_state = capture(
        destination.backend,
        destination_path,
        destination_expected,
        "copy destination",
    )
    .map_err(AttemptError::pre_commit)?;
    let mut staged = super::apply_stage::stage(
        source,
        source_path,
        &source_state,
        source_expected,
        destination.backend,
        destination_path,
        &destination_state,
        durability,
        throttle,
        cancel,
        bytes,
    )
    .map_err(AttemptError::pre_commit)?;
    gate(cancel, sink).map_err(AttemptError::pre_commit)?;
    revalidate(
        destination.backend,
        destination_path,
        &destination_state,
        "copy destination",
    )
    .map_err(AttemptError::pre_commit)?;
    super::apply_boundary::guard(
        destination.backend,
        destination.root,
        rel,
        opts.cross_mounts,
    )
    .map_err(AttemptError::pre_commit)?;
    let preserved = if destination_state.metadata.is_some() && opts.reversible {
        let versions = versions.ok_or_else(|| {
            AttemptError::pre_commit(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reversible copy has no run versions",
            ))
        })?;
        progress(super::apply_transfer::CopyReplacePhase::BackingUp);
        Some(
            version_save::save(
                versions,
                destination,
                destination_path,
                rel,
                &destination_state,
                destination_expected,
                VersionReason::Replaced,
                cancel,
            )
            .map_err(AttemptError::commit_attempted)?,
        )
    } else {
        if destination_state.metadata.is_some() {
            super::apply_transfer::verify_expected_content(
                destination.backend,
                destination_path,
                &destination_state,
                destination_expected,
                cancel,
            )
            .map_err(AttemptError::pre_commit)?;
        }
        None
    };
    let result = (|| {
        gate(cancel, sink)?;
        // Never read the source after publication. Its captured identity
        // and streamed digest have both been checked before this boundary.
        revalidate(source, source_path, &source_state, "copy source")?;
        super::apply_boundary::guard(
            destination.backend,
            destination.root,
            rel,
            opts.cross_mounts,
        )?;
        let current = if preserved.as_ref().is_some_and(|version| version.moved) {
            CapturedFile { metadata: None }
        } else {
            destination_state
        };
        progress(super::apply_transfer::CopyReplacePhase::Copying);
        if current.metadata.is_some()
            && !destination.backend.has_duplicate_file_names()
            && !destination
                .backend
                .mount_path_capabilities(destination_path)?
                .staged_write
                .namespace_replace
        {
            let versions = versions.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "provider replacement requires recorded run versions",
                )
            })?;
            staged.bind(versions, destination, rel, sink.is_some())?;
            if let Some(backup) = &preserved {
                staged.require_backup(backup.signature)?;
            } else if let ExpectedFile::Present(signature) = destination_expected {
                if signature.hash != 0 {
                    staged.require_backup(signature)?;
                }
            }
        }
        staged.publish(destination_path, &current, opts.verify, cancel)
    })();
    match result {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            if let Some(preserved) = preserved.as_ref() {
                if let Err(rollback) =
                    version_save::rollback(destination, destination_path, preserved)
                {
                    return Err(AttemptError::commit_attempted(io::Error::other(format!(
                        "copy failed ({error}); restoring the archived original failed: {rollback}"
                    ))));
                }
            }
            Err(AttemptError::commit_attempted(error))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn delete(
    side: &VersionSide<'_>,
    path: &str,
    expected: ExpectedFile,
    rel: &str,
    opts: BisyncOptions,
    versions: Option<&RunVersions>,
    cancel: &AtomicBool,
    sink: Option<&dyn ApplySink>,
    mut guard: impl FnMut() -> io::Result<()>,
) -> Result<bool, AttemptError> {
    gate(cancel, sink).map_err(AttemptError::pre_commit)?;
    super::apply_boundary::guard(side.backend, side.root, rel, opts.cross_mounts)
        .map_err(AttemptError::pre_commit)?;
    let captured =
        capture(side.backend, path, expected, "delete target").map_err(AttemptError::pre_commit)?;
    let Some(meta) = captured.metadata.as_ref() else {
        return super::apply_stage::namespace(side.backend, path).map_err(AttemptError::pre_commit);
    };
    let preserved: Option<Preserved> = if opts.reversible {
        let versions = versions.ok_or_else(|| {
            AttemptError::pre_commit(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reversible delete has no run versions",
            ))
        })?;
        // Recycle retains its established semantics; an archived rename
        // would leave nothing for the OS trash. Copy a private backup first.
        if opts.use_recycle {
            let mut context = versions.context().clone();
            context.location = super::types::VersionsLocation::AppData;
            let backup_versions =
                RunVersions::with_app_data(context, versions.app_data_dir().to_path_buf());
            backup_versions
                .bind_lock(&versions.lock_id())
                .map_err(AttemptError::pre_commit)?;
            Some(
                version_save::save(
                    &backup_versions,
                    side,
                    path,
                    rel,
                    &captured,
                    expected,
                    VersionReason::Deleted,
                    cancel,
                )
                .map_err(AttemptError::pre_commit)?,
            )
        } else {
            Some(
                version_save::save(
                    versions,
                    side,
                    path,
                    rel,
                    &captured,
                    expected,
                    VersionReason::Deleted,
                    cancel,
                )
                .map_err(AttemptError::commit_attempted)?,
            )
        }
    } else {
        super::apply_transfer::verify_expected_content(
            side.backend,
            path,
            &captured,
            expected,
            cancel,
        )
        .map_err(AttemptError::pre_commit)?;
        None
    };
    let result = (|| {
        gate(cancel, sink)?;
        guard()?;
        super::apply_boundary::guard(side.backend, side.root, rel, opts.cross_mounts)?;
        if preserved.as_ref().is_some_and(|version| version.moved) {
            if side.backend.try_exists(path)? {
                return Err(super::apply_guard::drift(
                    "delete target was recreated after archival",
                ));
            }
        } else {
            revalidate(side.backend, path, &captured, "delete target")?;
            gate(cancel, sink)?;
            if opts.use_recycle && side.backend.is_local() {
                super::apply_delete::recycle_local(path)?;
            } else if opts.use_recycle && crate::vfs::supports_recycle(side.backend, path)? {
                match crate::vfs::recycle(
                    side.backend,
                    path,
                    &crate::vfs::RecycleExpectation {
                        size: meta.size,
                        sha256: None,
                    },
                )? {
                    crate::vfs::RecycleOutcome::Recycled => {}
                    crate::vfs::RecycleOutcome::Changed => {
                        return Err(super::apply_guard::drift("recycle target changed"))
                    }
                }
            } else {
                side.backend.remove_file_id(path, meta.id.as_deref())?;
            }
        }
        super::apply_stage::namespace(side.backend, path)
    })();
    match result {
        Ok(durable) => Ok(durable),
        Err(error) => {
            if let Some(version) = preserved.as_ref() {
                if let Err(rollback) = version_save::rollback(side, path, version) {
                    return Err(AttemptError::commit_attempted(io::Error::other(
                        format!("delete failed ({error}); restoring the archived original failed: {rollback}"))));
                }
            }
            Err(AttemptError::commit_attempted(error))
        }
    }
}
