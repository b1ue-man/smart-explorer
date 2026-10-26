//! Same-volume publication with a retained recovery source when observed.
use super::super::super::platform;
use super::super::durability::finish_direct_move;
use super::super::move_guard::{
    moved_target_changed_error, remove_quarantine, restore_after_error, restore_quarantine_if_any,
    source_changed_error, source_snapshot_file, source_snapshot_path, QuarantinedSource,
};
use super::super::publication::{self, PublicationObserver};
use super::super::staging::LocalFailure;
use super::{metadata_if_exists, next_name, LocalOutcome, LocalRequest};
use crate::types::Conflict;
use std::{
    io,
    path::{Path, PathBuf},
};

/// `None` falls back to the staged cross-device/capability path.
pub(super) fn try_move(
    request: &LocalRequest<'_>,
    _source_path: &Path,
    requested: &Path,
    target: &mut PathBuf,
    races: &mut usize,
    quarantine: &mut Option<QuarantinedSource>,
    observer: Option<&dyn PublicationObserver>,
) -> Result<Option<LocalOutcome>, LocalFailure> {
    // Retain a hardlink where available. Filesystems such as FAT/exFAT can
    // still rename the held quarantine itself without copying its contents.
    // Their live object proof does not become a persistent recovery identity.
    let source = quarantine.as_ref().ok_or_else(missing_quarantine)?;
    let alias = PublicationAlias::create(source)
        .map_err(|error| LocalFailure::Source(restore_after_error(quarantine.as_ref(), error)))?;
    let rename_source = alias
        .as_ref()
        .map_or(source.path.as_path(), |alias| alias.path.as_path());
    let source = quarantine.as_ref().ok_or_else(missing_quarantine)?;
    let live =
        match crate::copy::publication_proof::open_live_move_source(rename_source, source.file()) {
            Ok(file) => file,
            // A readonly source or unavailable flush rights keeps the established
            // copy-to-writable-stage path available, before the rename starts.
            Err(_) => return Ok(None),
        };
    loop {
        if request.cancel.load(std::sync::atomic::Ordering::Acquire) {
            drop(live);
            restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
            return Ok(Some(LocalOutcome::Canceled));
        }
        if let Err(error) = publication::before(observer, rename_source, target) {
            drop(live);
            if let Err(uncertain) = publication::preparation_failed(
                observer,
                rename_source,
                target,
                &error,
                quarantine.as_ref(),
            ) {
                if let Some(alias) = &alias {
                    alias.retain();
                }
                return Err(LocalFailure::Publish(uncertain));
            }
            return Err(LocalFailure::Publish(restore_after_error(
                quarantine.as_ref(),
                error,
            )));
        }
        if request.cancel.load(std::sync::atomic::Ordering::Acquire) {
            let error = io::Error::new(
                io::ErrorKind::Interrupted,
                "direct move canceled before publication",
            );
            if let Err(uncertain) = publication::preparation_failed(
                observer,
                rename_source,
                target,
                &error,
                quarantine.as_ref(),
            ) {
                if let Some(alias) = &alias {
                    alias.retain();
                }
                return Err(LocalFailure::Publish(uncertain));
            }
            drop(live);
            restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
            return Ok(Some(LocalOutcome::Canceled));
        }
        let committed = crate::copy::publication_proof::preflight_direct_move(&live, target)
            .and_then(|()| {
                platform::move_file(
                    rename_source,
                    target,
                    request.conflict == Conflict::Overwrite,
                )
            });
        #[cfg(test)]
        let committed = compatibility_tests::move_reply(committed);
        if let Err(error) = &committed {
            if let Err(uncertain) = publication::commit_failed(
                observer,
                rename_source,
                target,
                &live,
                error,
                alias.as_ref().and(quarantine.as_ref()),
            ) {
                if let Some(alias) = &alias {
                    alias.retain();
                }
                return Err(LocalFailure::Publish(uncertain));
            }
        }
        match committed {
            Ok(()) => {
                let source = quarantine.as_ref().ok_or_else(missing_quarantine)?;
                validate_target(source, target, &live, alias.is_some()).map_err(|error| {
                    LocalFailure::Publish(moved_target_changed_error(target, Some(error)))
                })?;
                // A durable receipt records publication; it does not itself
                // flush destination bytes or the destination directory entry.
                // Confirm both before destroying any retained source hardlink.
                crate::copy::publication_proof::confirm_handle_publication(
                    target, &live, request.cancel,
                ).map_err(|error| LocalFailure::Publish(io::Error::new(error.kind(), format!(
                    "move committed at {}; {}; destination durability confirmation failed: {error}",
                    target.display(), if alias.is_some() { format!("secured source retained at {}", source.path.display()) }
                    else { "moved source remains at the destination; no source cleanup was performed".into() },
                ))))?;
                publication::after(
                    observer,
                    target,
                    &live,
                    alias.as_ref().and(quarantine.as_ref()),
                )
                .map_err(LocalFailure::Publish)?;
                // Release the write reservation before the conditional DELETE
                // handle used by Windows quarantine cleanup is acquired.
                drop(live);
                // Also handles POSIX rename's no-op for two hardlinks to
                // the same inode. The retained source is never cleaned up
                // before the publication receipt is durable.
                if alias.is_some()
                    && metadata_if_exists(&source.path)
                        .map_err(LocalFailure::Publish)?
                        .is_some()
                {
                    remove_quarantine(source).map_err(LocalFailure::Publish)?;
                }
                finish_direct_move(target, source, platform::sync_parent)
                    .map_err(LocalFailure::Publish)?;
                let bytes = source.snapshot.len;
                quarantine.take();
                return Ok(Some(LocalOutcome::Completed {
                    bytes,
                    target: target.clone(),
                }));
            }
            Err(error) if platform::is_cross_device(&error) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match request.conflict {
                Conflict::Skip => {
                    drop(live);
                    restore_quarantine_if_any(quarantine.as_ref()).map_err(LocalFailure::Source)?;
                    return Ok(Some(LocalOutcome::Skipped));
                }
                Conflict::Rename => {
                    *target = match next_name(requested, races) {
                        Ok(next) => next,
                        Err(error) => {
                            drop(live);
                            return Err(LocalFailure::Publish(restore_after_error(
                                quarantine.as_ref(),
                                error,
                            )));
                        }
                    };
                }
                Conflict::Overwrite => {
                    drop(live);
                    return Err(LocalFailure::Publish(restore_after_error(
                        quarantine.as_ref(),
                        error,
                    )));
                }
            },
            Err(error) => {
                drop(live);
                return Err(LocalFailure::Publish(restore_after_error(
                    quarantine.as_ref(),
                    error,
                )));
            }
        }
    }
}

fn validate_target(
    source: &QuarantinedSource,
    target: &Path,
    live: &std::fs::File,
    retained: bool,
) -> io::Result<()> {
    let reached =
        crate::local_access::open_regular(target, crate::local_access::FinalLink::Refuse)?;
    let current = source_snapshot_file(&reached)?;
    let held = source_snapshot_file(live)?;
    // FAT file numbers can change on a cross-directory rename. Both live
    // handles must identify the same current object; size/time alone do not.
    if !platform::same_file_object(&reached, live)?
        || current != held
        || current.len != source.snapshot.len
        || current.modified != source.snapshot.modified
        || retained && current != source.snapshot
    {
        return Err(source_changed_error(target));
    }
    Ok(())
}

fn missing_quarantine() -> LocalFailure {
    LocalFailure::Publish(io::Error::new(
        io::ErrorKind::InvalidData,
        "move quarantine is missing",
    ))
}

struct PublicationAlias {
    path: PathBuf,
    identity: Option<String>,
    file: std::fs::File,
    parent: crate::local_access::ConfinedDirectory,
    discard_on_drop: std::cell::Cell<bool>,
}

impl PublicationAlias {
    fn create(source: &QuarantinedSource) -> io::Result<Option<Self>> {
        #[cfg(test)]
        if compatibility_tests::without_hardlinks() {
            return Ok(None);
        }
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hash, Hasher};
        let parent = source.path.parent().unwrap_or_else(|| Path::new("."));
        let name = source
            .path
            .file_name()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "move quarantine has no file name",
                )
            })?
            .to_string_lossy();
        let identity = match publication::local_file_identity_handle(source.file()) {
            Ok(identity) => Some(identity),
            Err(error) if error.kind() == io::ErrorKind::Unsupported => None,
            Err(error) => return Err(error),
        };
        for attempt in 0..1000u32 {
            let mut random = RandomState::new().build_hasher();
            source.path.hash(&mut random);
            attempt.hash(&mut random);
            let tail = format!(".se-move-publish-{:016x}", random.finish());
            let path = parent.join(crate::vfs::fit_stage_name("", &name, &tail));
            let file = source.file().try_clone()?;
            match platform::hard_link(&source.path, &path) {
                Ok(()) => {
                    let alias = Self {
                        path,
                        identity,
                        file,
                        parent: source.parent().clone(),
                        discard_on_drop: std::cell::Cell::new(true),
                    };
                    let alias_name = alias
                        .path
                        .file_name()
                        .ok_or_else(|| source_changed_error(&source.path))?;
                    let actual = alias.parent.entry(alias_name)?;
                    if source_snapshot_path(&alias.path)? != source.snapshot
                        || source_snapshot_path(&source.path)? != source.snapshot
                        || !platform::same_file_object(&actual, source.file())?
                    {
                        return Err(source_changed_error(&source.path));
                    }
                    if let Some(identity) = &alias.identity {
                        if publication::local_file_identity(&alias.path)? != *identity {
                            return Err(source_changed_error(&source.path));
                        }
                    }
                    return Ok(Some(alias));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                // A failed hardlink is not a cross-volume move. The caller
                // may rename the held quarantine and keep its outcome honest.
                Err(_) => return Ok(None),
            }
        }
        Ok(None)
    }

    fn retain(&self) {
        self.discard_on_drop.set(false);
    }
}

#[cfg(test)]
#[path = "batches_direct_move_compatibility_task_tests.rs"]
mod compatibility_tests;

impl Drop for PublicationAlias {
    fn drop(&mut self) {
        if !self.discard_on_drop.get() {
            return;
        }
        let _ = super::super::recovery::discard_transfer_stage_live(
            &self.parent,
            &self.path,
            &self.file,
        );
    }
}
