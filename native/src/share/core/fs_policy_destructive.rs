//! A directory rename must not carry protected children past the path guard.
use std::{io, path::PathBuf};
use crate::local_access::{DirectoryHandle, LocalEntry};
use super::{TargetPolicy, private_name};

struct Frame {
    handle: DirectoryHandle,
    entries: Box<dyn Iterator<Item = io::Result<LocalEntry>> + Send>,
    path: PathBuf,
    bytes: u64,
}

pub(super) fn check(policy: &TargetPolicy, path: &str) -> io::Result<()> {
    let root = DirectoryHandle::open_root(&super::super::fs::local_paths::to_os_path(path))?;
    let budget = crate::transfer::memory_budget() / 8;
    let mut used = 0;
    let mut stack = vec![frame(root, PathBuf::from(path), budget, &mut used)?];
    while let Some(top) = stack.last_mut() {
        if let Some(entry) = top.entries.next() {
            let entry = entry?;
            if private_name(&entry.name.to_string_lossy()) { return Err(super::denied()); }
            let child = top.path.join(&entry.name);
            policy.write(&child.to_string_lossy())?;
            if entry.is_dir && !entry.is_link_like {
                let handle = top.handle.open_child(&entry.name)?;
                stack.push(frame(handle, child, budget, &mut used)?);
            }
        } else {
            let Some(frame) = stack.pop() else { break };
            used = used.saturating_sub(frame.bytes);
        }
    }
    Ok(())
}

fn frame(handle: DirectoryHandle, path: PathBuf, budget: u64, used: &mut u64) -> io::Result<Frame> {
    super::ensure_handle_allowed(&handle)?;
    // Includes the platform's bounded enumeration buffer and retained path.
    let bytes = 128 * 1024 + (path.as_os_str().len() as u64).saturating_mul(4);
    *used = used.checked_add(bytes).filter(|used| *used <= budget)
        .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "Schutzprüfung erreicht ihr Speicherbudget"))?;
    let entries = Box::new(handle.read_directory()?);
    Ok(Frame { handle, entries, path, bytes })
}
