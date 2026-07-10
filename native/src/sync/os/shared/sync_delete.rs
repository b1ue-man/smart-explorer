use crate::vfs::{Backend, VfsMeta};
use std::collections::{HashSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::imp::{join, record_error, rel_of, require_plain_directory, SyncStats, WalkBudget};

#[derive(Clone)]
struct Candidate {
    path: String,
    source_path: String,
    metadata: VfsMeta,
}

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
) {
    let mut files = Vec::new();
    let mut directories = Vec::new();
    if let Err(error) = collect_candidates(
        source,
        source_root,
        destination,
        destination_root,
        cancel,
        &mut files,
        &mut directories,
    ) {
        record_error(
            stats,
            errors,
            destination_root,
            format!("mirror deletion preflight failed; nothing deleted: {error}"),
        );
        return;
    }
    if let Err(error) = revalidate_plan(source, destination, &files, &directories, cancel) {
        record_error(
            stats,
            errors,
            destination_root,
            format!("mirror deletion revalidation failed; nothing deleted: {error}"),
        );
        return;
    }
    if dry_run {
        stats.deleted = stats
            .deleted
            .saturating_add((files.len() + directories.len()) as u64);
        return;
    }

    for candidate in &files {
        if let Err(error) = delete_file(source, destination, candidate, cancel) {
            record_error(stats, errors, &candidate.path, error.to_string());
        } else {
            stats.deleted = stats.deleted.saturating_add(1);
        }
    }
    directories.sort_by_key(|candidate| std::cmp::Reverse(candidate.path.len()));
    for candidate in &directories {
        if let Err(error) = delete_directory(source, destination, candidate, cancel) {
            record_error(stats, errors, &candidate.path, error.to_string());
        } else {
            stats.deleted = stats.deleted.saturating_add(1);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_candidates(
    source: &dyn Backend,
    source_root: &str,
    destination: &dyn Backend,
    destination_root: &str,
    cancel: &AtomicBool,
    files: &mut Vec<Candidate>,
    directories: &mut Vec<Candidate>,
) -> io::Result<()> {
    require_plain_directory(source, source_root, false)?;
    require_plain_directory(destination, destination_root, false)?;
    let mut queue = VecDeque::from([(destination_root.to_string(), 0usize)]);
    let mut budget = WalkBudget::default();
    budget
        .record(destination_root, 0)
        .map_err(io::Error::other)?;
    while let Some((directory, depth)) = queue.pop_front() {
        check_cancel(cancel)?;
        require_plain_directory(destination, &directory, false)?;
        let entries = destination.list_dir(&directory)?;
        let mut names = HashSet::new();
        for metadata in entries {
            check_cancel(cancel)?;
            crate::vfs::validate_child_name(&metadata.name)?;
            if !names.insert(metadata.name.clone()) {
                return Err(invalid(format!(
                    "destination returned a duplicate child name in {directory}: {:?}",
                    metadata.name
                )));
            }
            let path = join(&directory, &metadata.name);
            budget.record(&path, depth + 1).map_err(io::Error::other)?;
            let rel = rel_of(&path, destination_root);
            let source_path = join(source_root, &rel);
            let absent = source_absent(source, &source_path)?;
            let candidate = Candidate {
                path: path.clone(),
                source_path,
                metadata: metadata.clone(),
            };
            // Link-like directories are leaf entries and are never traversed.
            if metadata.is_dir && !metadata.is_symlink {
                queue.push_back((path, depth + 1));
                if absent {
                    directories.push(candidate);
                }
            } else if absent {
                files.push(candidate);
            }
        }
    }
    Ok(())
}

fn revalidate_plan(
    source: &dyn Backend,
    destination: &dyn Backend,
    files: &[Candidate],
    directories: &[Candidate],
    cancel: &AtomicBool,
) -> io::Result<()> {
    for candidate in files.iter().chain(directories) {
        check_cancel(cancel)?;
        if !source_absent(source, &candidate.source_path)? {
            return Err(invalid(format!(
                "source appeared during mirror preflight: {}",
                candidate.source_path
            )));
        }
        let current = destination.stat(&candidate.path)?;
        if !same_entry(&candidate.metadata, &current) {
            return Err(invalid(format!(
                "destination changed during mirror preflight: {}",
                candidate.path
            )));
        }
    }
    Ok(())
}

fn delete_file(
    source: &dyn Backend,
    destination: &dyn Backend,
    candidate: &Candidate,
    cancel: &AtomicBool,
) -> io::Result<()> {
    check_cancel(cancel)?;
    ensure_still_extra(source, candidate)?;
    let current = destination.stat(&candidate.path)?;
    if !same_entry(&candidate.metadata, &current) || (current.is_dir && !current.is_symlink) {
        return Err(invalid("extra file changed before deletion; retained"));
    }
    destination.remove_file_id(
        &candidate.path,
        candidate.metadata.id.as_deref().or(current.id.as_deref()),
    )
}

fn delete_directory(
    source: &dyn Backend,
    destination: &dyn Backend,
    candidate: &Candidate,
    cancel: &AtomicBool,
) -> io::Result<()> {
    check_cancel(cancel)?;
    ensure_still_extra(source, candidate)?;
    let current = destination.stat(&candidate.path)?;
    if current.is_symlink || !current.is_dir || !same_identity(&candidate.metadata, &current) {
        return Err(invalid("extra directory changed before deletion; retained"));
    }
    destination.remove_dir(&candidate.path)
}

fn ensure_still_extra(source: &dyn Backend, candidate: &Candidate) -> io::Result<()> {
    if source_absent(source, &candidate.source_path)? {
        Ok(())
    } else {
        Err(invalid(format!(
            "source appeared before deletion; retained: {}",
            candidate.source_path
        )))
    }
}

fn source_absent(source: &dyn Backend, path: &str) -> io::Result<bool> {
    match source.stat(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn same_entry(expected: &VfsMeta, current: &VfsMeta) -> bool {
    expected.is_dir == current.is_dir
        && expected.is_symlink == current.is_symlink
        && same_identity(expected, current)
        && (expected.is_dir
            || (expected.size == current.size && expected.mtime_ms == current.mtime_ms))
}

fn same_identity(expected: &VfsMeta, current: &VfsMeta) -> bool {
    match (expected.id.as_deref(), current.id.as_deref()) {
        (Some(left), Some(right)) => left == right,
        (None, None) => true,
        _ => false,
    }
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "mirror deletion canceled",
        ))
    } else {
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
