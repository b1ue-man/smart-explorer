//! Preserve provider operations behind the export and live session boundary.
use std::{io::{self, Read, Write}, sync::Arc};
use crate::vfs::{self, Backend, BackendHandle, BackendExtensions, Scheme, VfsMeta, VfsResult};
use super::{fs_access::AccessAuthority, fs_policy::{private_name, TargetPolicy}};

#[path = "fs_guard_extensions.rs"]
mod extensions;
#[path = "fs_guard_reports.rs"]
mod reports;
#[path = "fs_guard_stream.rs"]
mod stream;
#[path = "fs_guard_bulk.rs"]
mod bulk;
#[path = "fs_guard_names.rs"]
mod names;

pub(super) struct GuardedBackend {
    inner: BackendHandle,
    policy: TargetPolicy,
    authority: Option<Arc<AccessAuthority>>,
}

impl GuardedBackend {
    pub(super) fn new(inner: BackendHandle, policy: TargetPolicy, authority: Option<Arc<AccessAuthority>>) -> Self {
        Self { inner, policy, authority }
    }
    fn read(&self, path: &str) -> io::Result<()> {
        if let Some(authority) = &self.authority { authority.check()?; }
        names::check(self.inner.scheme(), path)?;
        self.policy.read(path)
    }
    fn write(&self, path: &str) -> io::Result<()> {
        if let Some(authority) = &self.authority { authority.check_write()?; }
        names::check(self.inner.scheme(), path)?;
        self.policy.write(path)
    }
    fn destructive(&self, path: &str) -> io::Result<()> {
        self.write(path)?;
        self.policy.destructive(path)
    }
    fn reader(&self, inner: Box<dyn Read + Send>) -> Box<dyn Read + Send> {
        Box::new(GuardedReader { inner, authority: self.authority.clone() })
    }
    fn writer(&self, inner: Box<dyn Write + Send>) -> Box<dyn Write + Send> {
        Box::new(GuardedWriter { inner, authority: self.authority.clone() })
    }
    fn filter(&self, path: &str, entries: Vec<VfsMeta>) -> Vec<VfsMeta> {
        entries.into_iter().filter(|entry| {
            if private_name(&entry.name) { return false; }
            let child = format!("{}/{}", path.trim_end_matches('/'), entry.name);
            // Links need an effective-target check; plain entries only need
            // the lexical private-root check, without another stat per item.
            self.policy.visible(&child, entry.is_symlink)
        }).collect()
    }
}

macro_rules! read_delegate {
    ($name:ident($($arg:ident:$ty:ty),*) -> $out:ty; $path:ident) => {
        fn $name(&self, $($arg:$ty),*) -> VfsResult<$out> {
            self.read($path)?; self.inner.$name($($arg),*)
        }
    }
}
macro_rules! write_delegate {
    ($name:ident($($arg:ident:$ty:ty),*) -> $out:ty; $($path:ident),+) => {
        fn $name(&self, $($arg:$ty),*) -> VfsResult<$out> {
            $(self.write($path)?;)+ self.inner.$name($($arg),*)
        }
    }
}
macro_rules! destructive_delegate {
    ($name:ident($($arg:ident:$ty:ty),*); $($path:ident),+) => {
        fn $name(&self, $($arg:$ty),*) -> VfsResult<()> {
            $(self.destructive($path)?;)+ self.inner.$name($($arg),*)
        }
    }
}

impl Backend for GuardedBackend {
    fn scheme(&self) -> Scheme { self.inner.scheme() }
    fn root_display(&self) -> String { self.inner.root_display() }
    fn state_identity(&self) -> String { self.inner.state_identity() }
    fn namespace_identity(&self) -> String { self.inner.namespace_identity() }
    // Never expose the raw inner backend through a cache escape hatch.
    fn extensions(&self) -> Option<&dyn BackendExtensions> { Some(self) }
    fn is_local(&self) -> bool { self.inner.is_local() }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.read(path)?; Ok(self.filter(path, self.inner.list_dir(path)?))
    }
    fn list_dir_for_sync(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.read(path)?; Ok(self.filter(path, self.inner.list_dir_for_sync(path)?))
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.read(path)?; Ok(self.reader(self.inner.open_read(path)?))
    }
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.read(path)?; Ok(self.reader(self.inner.open_read_id(path, id)?))
    }
    fn open_read_at(&self, path: &str, id: Option<&str>, offset: u64) -> VfsResult<Option<Box<dyn Read + Send>>> {
        self.read(path)?; Ok(self.inner.open_read_at(path, id, offset)?.map(|reader| self.reader(reader)))
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.write(path)?; Ok(self.writer(self.inner.open_write(path)?))
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.write(path)?; Ok(self.writer(self.inner.open_write_new(path)?))
    }
    fn open_write_copy_stage(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.write(path)?; Ok(self.writer(self.inner.open_write_copy_stage(path)?))
    }
    fn open_write_copy_stage_sized(&self, path: &str, size: u64) -> VfsResult<Box<dyn Write + Send>> {
        self.write(path)?; Ok(self.writer(self.inner.open_write_copy_stage_sized(path, size)?))
    }
    fn open_write_copy_stage_unsynced(&self, path: &str, size: u64) -> VfsResult<Box<dyn Write + Send>> {
        self.write(path)?; Ok(self.writer(self.inner.open_write_copy_stage_unsynced(path, size)?))
    }
    fn open_write_fresh(&self, path: &str, size: u64) -> VfsResult<Option<Box<dyn Write + Send>>> {
        self.write(path)?; Ok(self.inner.open_write_fresh(path, size)?.map(|writer| self.writer(writer)))
    }
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        self.read(src)?; self.write(dst)?; vfs::copy_between(self, src, self, dst)
    }
    fn server_copy_to_stage(&self, src: &str, stage: &str, size: u64, cancel: &std::sync::atomic::AtomicBool) -> VfsResult<Option<u64>> {
        self.read(src)?; self.write(stage)?;
        let copied = stream::cancellable(self.authority.as_ref(), cancel,
            |cancelled| self.inner.server_copy_to_stage(src, stage, size, cancelled))?;
        self.read(src)?; self.write(stage)?;
        Ok(copied)
    }
    read_delegate!(stat(path: &str) -> VfsMeta; path);
    read_delegate!(try_exists(path: &str) -> bool; path);
    read_delegate!(item_id(path: &str) -> Option<String>; path);
    read_delegate!(read_size(path: &str, size: u64) -> Option<u64>; path);
    destructive_delegate!(rename(src: &str, dst: &str); src, dst);
    destructive_delegate!(rename_no_replace(src: &str, dst: &str); src, dst);
    destructive_delegate!(promote_staged(staged: &str, destination: &str); staged, destination);
    destructive_delegate!(promote_staged_to_id(staged: &str, destination: &str, id: Option<&str>); staged, destination);
    destructive_delegate!(promote_staged_no_replace(staged: &str, destination: &str); staged, destination);
    destructive_delegate!(promote_copy_stage(staged: &str, destination: &str); staged, destination);
    write_delegate!(remove_file(path: &str) -> (); path);
    write_delegate!(remove_file_id(path: &str, id: Option<&str>) -> (); path);
    destructive_delegate!(remove_dir(path: &str); path);
    write_delegate!(mkdir_all(path: &str) -> (); path);
    write_delegate!(create_dir(path: &str) -> (); path);
    write_delegate!(create_dir_new(path: &str) -> (); path);
    write_delegate!(discard_copy_stage(stage: &str) -> (); stage);
    fn has_duplicate_file_names(&self) -> bool { self.inner.has_duplicate_file_names() }
    fn transfer_hint(&self) -> Option<String> { self.inner.transfer_hint() }
    fn download_name(&self, path: &str, name: &str) -> String { self.inner.download_name(path, name) }
    fn parallelism(&self) -> usize { self.inner.parallelism() }
    fn flow_key(&self, path: &str) -> String { self.inner.flow_key(path) }
    fn transfer_ceiling(&self, path: &str) -> Option<usize> { self.inner.transfer_ceiling(path) }
    fn concurrent_read_write(&self) -> bool { self.inner.concurrent_read_write() }
    fn rename_overwrites(&self) -> bool { self.policy.access.allows_write() && self.inner.rename_overwrites() }
    fn staged_write_capabilities(&self, root: &str) -> vfs::StagedWriteCapabilities {
        if self.write(root).is_err() { vfs::StagedWriteCapabilities::default() } else { self.inner.staged_write_capabilities(root) }
    }
    fn mount_path_capabilities(&self, root: &str) -> VfsResult<vfs::MountPathCapabilities> {
        self.read(root)?;
        let mut capabilities = self.inner.mount_path_capabilities(root)?;
        if self.write(root).is_err() { capabilities.staged_write = vfs::StagedWriteCapabilities::default(); }
        Ok(capabilities)
    }
    fn case_sensitive_paths(&self, root: &str) -> bool { self.inner.case_sensitive_paths(root) }
    fn root_confinement(&self, root: &str) -> vfs::RootConfinement { self.inner.root_confinement(root) }
    fn delete_disposition(&self) -> vfs::DeleteDisposition { self.inner.delete_disposition() }
    fn provides_content_hash(&self) -> bool { self.inner.provides_content_hash() }
    fn supports_changes(&self) -> bool { self.inner.supports_changes() }
    read_delegate!(change_root_id(root: &str) -> Option<String>; root);
    read_delegate!(current_change_cursor(root: &str) -> Option<String>; root);
    fn changes_since(&self, root: &str, cursor: &str) -> VfsResult<vfs::VfsChangeBatch> {
        self.read(root)?;
        let mut changes = self.inner.changes_since(root, cursor)?;
        changes.changes.retain(|change| !change.rel.as_ref().is_some_and(|rel| super::fs_policy::private_path(rel))
            && !change.name.as_ref().is_some_and(|name| private_name(name)));
        Ok(changes)
    }
    fn invalidate_cache(&self) { self.inner.invalidate_cache(); }
    fn scan_storage(&self, root: &str, progress: &crate::analytics::Progress) -> VfsResult<Option<crate::analytics::ScanOutcome>> {
        self.read(root)?;
        if let Some(authority) = &self.authority { authority.register_cancel(&progress.cancel)?; }
        let outcome = self.inner.scan_storage(root, progress)?;
        self.read(root)?;
        outcome.map(|outcome| reports::scan(outcome, root, &self.policy)).transpose()
    }
    fn supports_walk_tree(&self) -> bool { self.inner.supports_walk_tree() }
    fn walk_tree(&self, root: &str, on_progress: &(dyn Fn(u64, u64) -> bool + Sync)) -> VfsResult<Option<crate::agent_proto::WireNode>> {
        self.read(root)?;
        let tree = self.inner.walk_tree(root, &|files, bytes| self.read(root).is_ok() && on_progress(files, bytes))?;
        self.read(root)?;
        tree.map(|node| reports::wire(node, root, &self.policy)).transpose()
    }
    fn supports_bulk_tree(&self) -> bool { self.inner.supports_bulk_tree() }
    fn get_tree(&self, root: &str, dst: &std::path::Path) -> VfsResult<u64> {
        self.read(root)?; bulk::get_tree(self, root, dst)
    }
    fn put_tree(&self, src: &std::path::Path, root: &str) -> VfsResult<u64> {
        self.write(root)?; bulk::put_tree(self, src, root)
    }
    fn batch_limits(&self, dir: &str) -> Option<vfs::BatchLimits> {
        if self.read(dir).is_err() { None } else { self.inner.batch_limits(dir) }
    }
    fn put_batch(&self, entries: &[vfs::BatchPut], data: &mut dyn Read) -> VfsResult<Vec<vfs::BatchPutOutcome>> {
        for entry in entries { self.write(&entry.path)?; }
        let mut guarded = bulk::Upload { inner: data, authority: self.authority.as_deref() };
        let outcome = self.inner.put_batch(entries, &mut guarded)?;
        for entry in entries { self.write(&entry.path)?; }
        Ok(outcome)
    }
    fn get_batch(&self, items: &[vfs::BatchGet], sink: &mut dyn vfs::BatchSink) -> VfsResult<()> {
        for item in items { self.read(&item.path)?; }
        self.inner.get_batch(items, &mut bulk::Download { inner: sink, authority: self.authority.as_deref() })
    }
    fn supports_search(&self) -> bool { self.inner.supports_search() }
    fn search(&self, root: &str, spec: &crate::agent_proto::SearchSpec,
        tx: crossbeam_channel::Sender<vfs::SearchHit>, cancel: &std::sync::atomic::AtomicBool) -> VfsResult<bool> {
        self.read(root)?;
        stream::forward(self.authority.as_ref(), &self.policy, root, tx, cancel,
            |hit| &hit.rel, |output, cancelled| self.inner.search(root, spec, output, cancelled))
    }
    fn supports_walk_hashed(&self) -> bool { self.inner.supports_walk_hashed() }
    fn walk_hashed(&self, root: &str, want_hash: bool, tx: crossbeam_channel::Sender<vfs::HashHit>,
        cancel: &std::sync::atomic::AtomicBool) -> VfsResult<bool> {
        self.read(root)?;
        stream::forward(self.authority.as_ref(), &self.policy, root, tx, cancel,
            |hit| &hit.rel, |output, cancelled| self.inner.walk_hashed(root, want_hash, output, cancelled))
    }
}

struct GuardedReader { inner: Box<dyn Read + Send>, authority: Option<Arc<AccessAuthority>> }
impl Read for GuardedReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if let Some(authority) = &self.authority { authority.check()?; }
        let read = self.inner.read(bytes)?;
        if let Some(authority) = &self.authority { authority.check()?; }
        Ok(read)
    }
}
struct GuardedWriter { inner: Box<dyn Write + Send>, authority: Option<Arc<AccessAuthority>> }
impl Write for GuardedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(authority) = &self.authority { authority.check_write()?; }
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(authority) = &self.authority { authority.check_write()?; }
        self.inner.flush()
    }
}

#[cfg(test)]
#[path = "fs_guard_backend_task_tests.rs"]
mod task_tests;
