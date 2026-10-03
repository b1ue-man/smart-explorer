//! Mirror removal is reversible and rechecks normalized source absence.
use super::imp::{record_error, SyncStats};
use super::sync_delete_walk::{self, Entry};
use crate::bisync::versions::RunVersions;
use crate::bisync::{KeyPolicy, OmissionKind, SyncOmissions};
use crate::vfs::Backend;
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

#[allow(clippy::too_many_arguments)]
pub(super) fn delete_extras(
    source: &dyn Backend,
    source_root: &str,
    destination: &dyn Backend,
    destination_root: &str,
    dry_run: bool,
    cancel: &AtomicBool,
    stats: &mut SyncStats,
    errors: &mut Vec<(String, String)>,
    omissions: &mut SyncOmissions,
) {
    let run =
        match super::sync_run::MirrorRun::begin(source, source_root, destination, destination_root)
        {
            Ok(run) => run,
            Err(error) => {
                record_error(stats, errors, destination_root, error.to_string());
                return;
            }
        };
    delete_extras_scoped(
        source,
        source_root,
        destination,
        destination_root,
        dry_run,
        cancel,
        stats,
        errors,
        omissions,
        &run.versions,
    );
    if let Err(error) = run.finish(destination, destination_root, cancel) {
        record_error(stats, errors, destination_root, error.to_string());
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn delete_extras_scoped(
    source: &dyn Backend,
    source_root: &str,
    destination: &dyn Backend,
    destination_root: &str,
    dry_run: bool,
    cancel: &AtomicBool,
    stats: &mut SyncStats,
    errors: &mut Vec<(String, String)>,
    omissions: &mut SyncOmissions,
    versions: &RunVersions,
) {
    let keys = KeyPolicy::for_pair(
        source.case_sensitive_paths(source_root),
        destination.case_sensitive_paths(destination_root),
    );
    let plan = (|| {
        let source_entries = sync_delete_walk::walk(source, source_root, keys, omissions, cancel)?;
        let present: HashSet<_> = source_entries
            .iter()
            .map(|entry| keys.key(&entry.rel).into_owned())
            .collect();
        let target_entries =
            sync_delete_walk::walk(destination, destination_root, keys, omissions, cancel)?;
        if omissions.contains("") {
            return Err(io::Error::other(
                "mirror root has a protected unreadable child",
            ));
        }
        Ok(target_entries
            .into_iter()
            .filter(|entry| {
                !present.contains(keys.key(&entry.rel).as_ref()) && !omissions.protects(&entry.rel)
            })
            .collect::<Vec<_>>())
    })();
    let mut candidates = match plan {
        Ok(candidates) => candidates,
        Err(error) => {
            record_error(
                stats,
                errors,
                destination_root,
                format!("mirror deletion preflight failed; nothing deleted: {error}"),
            );
            return;
        }
    };
    candidates.sort_by(|a, b| {
        a.meta
            .is_dir
            .cmp(&b.meta.is_dir)
            .then_with(|| b.rel.matches('/').count().cmp(&a.rel.matches('/').count()))
    });
    // Revalidate all targets and normalized source absences before any delete.
    for entry in &candidates {
        let validation = (|| -> io::Result<()> {
            if cancel.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "mirror canceled",
                ));
            }
            current(destination, entry)?;
            if sync_delete_walk::missing(source, source_root, &entry.rel, keys, cancel)? {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "mirror source appeared; all targets retained",
                ))
            }
        })();
        match validation {
            Ok(()) => {}
            Err(error) => {
                if let Some(kind) = crate::bisync::apply_boundary::omitted(&error) {
                    omissions.record_kind(&entry.rel, kind, kind.reported_by_default());
                }
                record_error(
                    stats,
                    errors,
                    &entry.path,
                    format!("mirror deletion revalidation failed; nothing deleted: {error}"),
                );
                return;
            }
        }
    }
    if dry_run {
        stats.deleted = stats.deleted.saturating_add(candidates.len() as u64);
        return;
    }
    for entry in candidates {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        if omissions.protects(&entry.rel) {
            continue;
        }
        let result = (|| {
            crate::bisync::apply_boundary::guard(destination, destination_root, &entry.rel, false)?;
            if entry.meta.is_dir {
                current_directory(destination, &entry)?;
            } else {
                current(destination, &entry)?;
            }
            let absent = || -> io::Result<()> {
                if sync_delete_walk::missing(source, source_root, &entry.rel, keys, cancel)? {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "mirror source appeared; target retained",
                    ))
                }
            };
            absent()?;
            if entry.meta.is_dir {
                let listing = crate::vfs::list_dir_tolerant(destination, &entry.path)?;
                if !listing.entries.is_empty() || !listing.omitted.is_empty() {
                    return Err(io::Error::new(
                        io::ErrorKind::DirectoryNotEmpty,
                        "mirror directory has protected or new children",
                    ));
                }
                absent()?;
                if cancel.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "mirror canceled",
                    ));
                }
                destination.remove_dir(&entry.path)?;
                if !crate::bisync::apply_stage::namespace(destination, &entry.path)? {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "mirror directory removal was not namespace-durable",
                    ));
                }
                Ok(())
            } else {
                crate::bisync::apply_transaction::quick_delete(
                    destination,
                    destination_root,
                    &entry.path,
                    &entry.rel,
                    super::sync_copy::signature(&entry.meta),
                    versions,
                    cancel,
                    absent,
                )
            }
        })();
        match result {
            Ok(()) => stats.deleted = stats.deleted.saturating_add(1),
            Err(error)
                if error.kind() == io::ErrorKind::Interrupted && cancel.load(Ordering::Acquire) =>
            {
                break
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
                ) || crate::bisync::apply_boundary::deferred(&error) =>
            {
                omissions.record_kind(&entry.rel, OmissionKind::Unreadable, true);
            }
            Err(error) => {
                if let Some(kind) = crate::bisync::apply_boundary::omitted(&error) {
                    omissions.record_kind(&entry.rel, kind, kind.reported_by_default());
                } else {
                    let terminal = crate::vfs::is_target_refusal(&error)
                        || matches!(
                            error.kind(),
                            io::ErrorKind::ConnectionReset
                                | io::ErrorKind::ConnectionAborted
                                | io::ErrorKind::NotConnected
                                | io::ErrorKind::BrokenPipe
                                | io::ErrorKind::HostUnreachable
                                | io::ErrorKind::NetworkUnreachable
                                | io::ErrorKind::ConnectionRefused
                                | io::ErrorKind::NetworkDown
                                | io::ErrorKind::TimedOut
                        );
                    record_error(stats, errors, &entry.path, error.to_string());
                    if terminal {
                        break;
                    }
                }
            }
        }
    }
}
fn current_directory(backend: &dyn Backend, entry: &Entry) -> io::Result<()> {
    let actual = backend.stat(&entry.path)?;
    // Removing our own children changes directory size/time. Its identity
    // and the following empty tolerant listing are the deletion boundary.
    if actual.is_dir && !actual.is_symlink && !actual.special && actual.id == entry.meta.id {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mirror directory identity changed",
        ))
    }
}
fn current(backend: &dyn Backend, entry: &Entry) -> io::Result<()> {
    let actual = backend.stat(&entry.path)?;
    if actual.is_dir == entry.meta.is_dir
        && !actual.is_symlink
        && !actual.special
        && actual.size == entry.meta.size
        && actual.mtime_ms == entry.meta.mtime_ms
        && actual.id == entry.meta.id
        && actual.content_md5 == entry.meta.content_md5
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mirror target changed since observation",
        ))
    }
}
