//! Depth-first deletion retains a bounded stack, never recursive Rust calls.
use crate::vfs::{Backend, VfsMeta};
use std::io;
#[path = "fs_delete_local.rs"]
mod local;

struct Directory {
    path: String,
    entries: std::vec::IntoIter<VfsMeta>,
    memory: usize,
}

pub(super) fn remove_tree(backend: &dyn Backend, root: &str) -> io::Result<()> {
    remove_tree_checked(backend, root, &|| Ok(()))
}

pub(super) fn remove_tree_checked(
    backend: &dyn Backend,
    root: &str,
    check_write: &dyn Fn() -> io::Result<()>,
) -> io::Result<()> {
    check_write()?;
    if backend.is_local() {
        return local::remove(backend, root, check_write);
    }
    let budget =
        usize::try_from((crate::transfer::memory_budget() / 8).max(4096)).unwrap_or(usize::MAX);
    let mut used = 0usize;
    let mut stack = vec![open(backend, root.to_owned(), budget, &mut used)?];
    while let Some(directory) = stack.last_mut() {
        if let Some(entry) = directory.entries.next() {
            check_write()?;
            crate::vfs::validate_child_name(&entry.name)?;
            let child = format!("{}/{}", directory.path.trim_end_matches('/'), entry.name);
            let current = backend.stat(&child)?;
            if current.is_symlink {
                // A link is one namespace entry, never another subtree.
                if current.is_dir {
                    backend.remove_dir(&child)?;
                } else {
                    backend.remove_file_id(&child, entry.id.as_deref())?;
                }
            } else if current.is_dir {
                stack.push(open(backend, child, budget, &mut used)?);
            } else {
                backend.remove_file_id(&child, entry.id.as_deref())?;
            }
        } else {
            check_write()?;
            let Some(directory) = stack.pop() else { break };
            // Revalidate the root after children: never call a backend's
            // recursive primitive, including when a directory became a link.
            let current = backend.stat(&directory.path)?;
            if !current.is_dir || current.is_symlink {
                return Err(changed());
            }
            backend.remove_dir(&directory.path)?;
            used = used.saturating_sub(directory.memory);
        }
    }
    Ok(())
}

fn open(
    backend: &dyn Backend,
    path: String,
    budget: usize,
    used: &mut usize,
) -> io::Result<Directory> {
    let metadata = backend.stat(&path)?;
    if !metadata.is_dir || metadata.is_symlink {
        return Err(changed());
    }
    let entries = backend.list_dir(&path)?;
    let retained = entries
        .capacity()
        .checked_mul(std::mem::size_of::<VfsMeta>())
        .and_then(|bytes| bytes.checked_add(path.capacity() + std::mem::size_of::<Directory>()))
        .ok_or_else(exhausted)?;
    let memory = entries
        .iter()
        .try_fold(retained, |sum, entry| {
            sum.checked_add(
                entry.name.capacity()
                    + entry.id.as_ref().map_or(0, String::capacity)
                    + entry.content_md5.as_ref().map_or(0, String::capacity),
            )
        })
        .ok_or_else(exhausted)?;
    *used = used
        .checked_add(memory)
        .filter(|total| *total <= budget)
        .ok_or_else(exhausted)?;
    Ok(Directory {
        path,
        entries: entries.into_iter(),
        memory,
    })
}
fn exhausted() -> io::Error {
    io::Error::new(
        io::ErrorKind::OutOfMemory,
        "Share-Löschlauf erreicht sein Speicherbudget; erneut versuchen",
    )
}
fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Löschziel ist kein unverändertes gewöhnliches Verzeichnis",
    )
}

#[cfg(test)]
#[path = "fs_delete_task_tests.rs"]
mod task_tests;
