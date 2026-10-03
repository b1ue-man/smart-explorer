//! Iterative deletion through held parents; child links are leaf entries.
use crate::{
    local_access::{DirectoryHandle, LocalEntry},
    vfs::Backend,
};
use std::{
    ffi::OsString,
    io,
    path::{Component, PathBuf},
};

struct Frame {
    handle: DirectoryHandle,
    entries: Box<dyn Iterator<Item = io::Result<LocalEntry>> + Send>,
    parent: DirectoryHandle,
    name: OsString,
    path: PathBuf,
    bytes: u64,
}

pub(super) fn remove(
    backend: &dyn Backend,
    target: &str,
    check_write: &dyn Fn() -> io::Result<()>,
) -> io::Result<()> {
    let root = super::super::fs::local_paths::to_os_path(&backend.root_display());
    let target = super::super::fs::local_paths::to_os_path(target);
    let relative = target.strip_prefix(&root).map_err(|_| refused())?;
    let mut names = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name.to_owned()),
            _ => Err(refused()),
        })
        .collect::<io::Result<Vec<_>>>()?;
    let name = names.pop().ok_or_else(refused)?; // Never remove the exported root.
    let mut parent = DirectoryHandle::open_root(&root)?;
    super::super::fs::ensure_local_share_handle_allowed(&parent)?;
    for name in names {
        parent = parent.open_child(&name)?;
        super::super::fs::ensure_local_share_handle_allowed(&parent)?;
    }
    let handle = parent.open_child_for_delete(&name)?;
    let budget = crate::transfer::memory_budget() / 8;
    let mut used = 0;
    let mut stack = vec![frame(handle, parent, name, target, budget, &mut used)?];
    while let Some(top) = stack.last_mut() {
        if let Some(entry) = top.entries.next() {
            check_write()?;
            let entry = entry?;
            if super::super::fs_policy::private_name(&entry.name.to_string_lossy()) {
                return Err(refused());
            }
            let child = top.path.join(&entry.name);
            // The live wrapper rechecks relation/export rights before every
            // mutation. Actual deletion is against the held parent, not path.
            backend.stat(&child.to_string_lossy().replace('\\', "/"))?;
            if entry.is_dir && !entry.is_link_like {
                let parent = top.handle.clone();
                let handle = parent.open_child_for_delete(&entry.name)?;
                stack.push(frame(handle, parent, entry.name, child, budget, &mut used)?);
            } else {
                top.handle.remove_child(&entry.name)?;
            }
        } else {
            check_write()?;
            let Some(frame) = stack.pop() else { break };
            backend.stat(&frame.path.to_string_lossy().replace('\\', "/"))?;
            let Frame {
                handle,
                entries,
                parent,
                name,
                bytes,
                ..
            } = frame;
            drop(entries); // Windows final disposition consumes the last child pin.
            parent.remove_empty_child(&name, handle)?;
            used = used.saturating_sub(bytes);
        }
    }
    Ok(())
}

fn frame(
    handle: DirectoryHandle,
    parent: DirectoryHandle,
    name: OsString,
    path: PathBuf,
    budget: u64,
    used: &mut u64,
) -> io::Result<Frame> {
    super::super::fs::ensure_local_share_handle_allowed(&handle)?;
    let bytes = 128 * 1024 + (path.as_os_str().len() as u64 + name.len() as u64).saturating_mul(4);
    *used = used
        .checked_add(bytes)
        .filter(|used| *used <= budget)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                "Share-Löschlauf erreicht sein Speicherbudget",
            )
        })?;
    let entries = Box::new(handle.read_directory()?);
    Ok(Frame {
        handle,
        entries,
        parent,
        name,
        path,
        bytes,
    })
}
fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Löschziel ist nicht freigegeben",
    )
}
