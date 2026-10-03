//! Bulk transfers retain provider batches; whole-tree copies enforce every path.
use super::{super::fs_access::AccessAuthority, GuardedBackend};
use crate::vfs::{self, Backend, BatchSink};
use std::{
    io::{self, Read},
    path::Path,
};

pub(super) struct Upload<'a> {
    pub(super) inner: &'a mut dyn Read,
    pub(super) authority: Option<&'a AccessAuthority>,
}
impl Read for Upload<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if let Some(authority) = self.authority {
            authority.check_write()?;
        }
        self.inner.read(bytes)
    }
}
pub(super) struct Download<'a> {
    pub(super) inner: &'a mut dyn BatchSink,
    pub(super) authority: Option<&'a AccessAuthority>,
}
impl Download<'_> {
    fn check(&self) -> io::Result<()> {
        if let Some(authority) = self.authority {
            authority.check()?;
        }
        Ok(())
    }
}
impl BatchSink for Download<'_> {
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()> {
        self.check()?;
        self.inner.begin(index, size)
    }
    fn data(&mut self, index: usize, bytes: &[u8]) -> io::Result<()> {
        self.check()?;
        self.inner.data(index, bytes)
    }
    fn end(&mut self, index: usize, result: io::Result<()>) -> io::Result<()> {
        self.check()?;
        self.inner.end(index, result)
    }
    fn failed(&mut self, index: usize, error: io::Error) -> io::Result<()> {
        self.check()?;
        self.inner.failed(index, error)
    }
}

pub(super) fn get_tree(source: &GuardedBackend, root: &str, dst: &Path) -> io::Result<u64> {
    let local = vfs::LocalBackend::new(&dst.to_string_lossy().replace('\\', "/"));
    copy_tree(source, root, &local, &local.root_display())
}
pub(super) fn put_tree(destination: &GuardedBackend, src: &Path, root: &str) -> io::Result<u64> {
    let local = vfs::LocalBackend::new(&src.to_string_lossy().replace('\\', "/"));
    let physical = local.root_display();
    let source = GuardedBackend::new(
        std::sync::Arc::new(local),
        super::super::fs_host_policy::TargetPolicy::new(
            super::super::export_config::ExportAccess::ReadOnly,
            false,
            true,
        )
        .with_root(&physical),
        destination.authority.clone(),
    );
    copy_tree(&source, &physical, destination, root)
}

fn copy_tree(
    source: &dyn Backend,
    root: &str,
    destination: &dyn Backend,
    target: &str,
) -> io::Result<u64> {
    let budget = crate::transfer::memory_budget() / 8;
    let mut pending = vec![(root.to_owned(), target.to_owned())];
    let overhead = std::mem::size_of::<(String, String)>() as u64;
    let mut retained = 2 * (root.len() + target.len()) as u64 + overhead;
    let mut count = 0u64;
    while let Some((from, to)) = pending.pop() {
        retained = retained.saturating_sub(2 * (from.len() + to.len()) as u64 + overhead);
        let current = source.stat(&from)?;
        if current.is_symlink || current.special {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Link ist keine rekursive Transferwurzel",
            ));
        }
        if !current.is_dir {
            vfs::copy_between(source, &from, destination, &to)?;
            count = count
                .checked_add(1)
                .ok_or_else(|| io::Error::other("Transferzähler erschöpft"))?;
            continue;
        }
        destination.mkdir_all(&to)?;
        let listing = vfs::list_dir_tolerant(source, &from)?;
        if !listing.omitted.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Baumtransfer enthält ausgelassene Einträge",
            ));
        }
        for entry in listing.entries {
            crate::vfs::validate_child_name(&entry.name)?;
            if entry.is_symlink || entry.special {
                continue;
            }
            let child = (
                format!("{}/{}", from.trim_end_matches('/'), entry.name),
                format!("{}/{}", to.trim_end_matches('/'), entry.name),
            );
            retained = retained
                .checked_add(2 * (child.0.len() + child.1.len()) as u64 + overhead)
                .filter(|bytes| *bytes <= budget)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::OutOfMemory,
                        "Baumtransfer erreicht sein Speicherbudget",
                    )
                })?;
            pending.push(child);
        }
    }
    Ok(count)
}
