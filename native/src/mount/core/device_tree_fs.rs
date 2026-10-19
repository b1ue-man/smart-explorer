//! The device tree as `MountFilesystem` (DRV-M6, U17-14 to U17-16): the
//! frontend boundary the Dokany and FUSE hosts use. Virtual folders answer
//! on their own and refuse every write; a path inside a shared folder goes
//! to the engine of that child. Handles and delete tokens of the tree are
//! its own counters, each holding the engine that issued the original.
use std::io;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::device_tree::DeviceTree;
use super::device_tree_backend::guard;
use super::device_tree_children::{ChildKey, Pin};
use super::device_tree_routing::{denied, Routed};
use super::frontend::{
    LockReadiness, MountFilesystem, PendingAttributes, PreloadProgress, StopProbe, VolumeFacts,
};
use super::metadata_cache::MetadataChange;
use super::path::ProjectedPath;
use super::path_dialect::PathDialect;
use super::types::{
    BackendRoot, Baseline, DeleteToken, EntryCondition, FlushOutcome, HandleId, MountConflict,
    NamespaceOutcome, OpenDisposition, OpenFileOptions, RenameOutcome,
};
use crate::vfs::VfsMeta;

fn invalid_handle() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "unbekannter Handle")
}

/// What opening a virtual folder or a share's top folder as a file means.
fn refuse_open(writable: bool, disposition: OpenDisposition) -> io::Error {
    if writable || disposition != OpenDisposition::OpenExisting {
        denied()
    } else {
        io::Error::new(io::ErrorKind::InvalidInput, "ein Ordner ist keine Datei")
    }
}

impl DeviceTree {
    fn add_handle(&self, pin: Pin, inner: HandleId) -> HandleId {
        let id = self.next_handle.fetch_add(1, Ordering::Relaxed);
        guard(&self.handles).insert(id, (pin, inner));
        HandleId(id)
    }

    fn with_handle<T>(
        &self,
        handle: HandleId,
        operation: impl FnOnce(&Pin, HandleId) -> io::Result<T>,
    ) -> io::Result<T> {
        let (pin, inner) = {
            let table = guard(&self.handles);
            let (pin, inner) = table.get(&handle.0).ok_or_else(invalid_handle)?;
            (pin.clone(), *inner)
        };
        operation(&pin, inner)
    }

    fn leaf(&self, callback_path: &str) -> String {
        self.segments(callback_path)
            .ok()
            .and_then(|segments| segments.last().cloned())
            .unwrap_or_default()
    }

    fn online(&self, key: &str) -> bool {
        guard(&self.state).online.get(key) == Some(&true)
    }

    /// A tree path for an absolute path inside the engine of a child.
    fn lift(prefix: &str, path: &str) -> String {
        if path == "/" {
            prefix.to_string()
        } else {
            format!("{prefix}{path}")
        }
    }

    fn prefix_of(&self, key: &ChildKey) -> Option<String> {
        self.child_prefix(key)
    }

    /// Both paths inside one child: the engine call runs; any other pair is
    /// refused (a virtual folder) or crosses shared folders.
    fn within_one_child<T>(
        &self,
        source: &str,
        destination: &str,
        operation: impl FnOnce(&Pin, &str, &str) -> io::Result<T>,
    ) -> io::Result<T> {
        let (Routed::Inside(from), Routed::Inside(to)) =
            (self.route(source)?, self.route_new(destination)?)
        else {
            return Err(denied());
        };
        if from.child != to.child {
            return Err(io::Error::new(
                io::ErrorKind::CrossesDevices,
                "Verschieben zwischen Freigaben ist nicht möglich; kopieren und danach löschen",
            ));
        }
        let pin = self.ensure_child(&from)?;
        operation(
            &pin,
            &from.engine_path(self.dialect),
            &to.engine_path(self.dialect),
        )
    }
}

impl MountFilesystem for DeviceTree {
    fn volume_facts(&self) -> VolumeFacts {
        VolumeFacts {
            id: self.id.clone(),
            root: BackendRoot::top(),
            dialect: self.dialect,
            case_sensitive: self.dialect == PathDialect::Posix,
            read_only: false,
        }
    }

    fn project(&self, callback_path: &str) -> io::Result<ProjectedPath> {
        self.projector.project(callback_path)
    }

    fn open_file(&self, callback_path: &str, options: OpenFileOptions) -> io::Result<HandleId> {
        let creates = options.writable || options.disposition != OpenDisposition::OpenExisting;
        let routed = if creates {
            self.route_new(callback_path)?
        } else {
            self.route(callback_path)?
        };
        match routed {
            Routed::Inside(child) => {
                let pin = self.ensure_child(&child)?;
                let inner = pin
                    .engine
                    .open_file(&child.engine_path(self.dialect), options)?;
                Ok(self.add_handle(pin, inner))
            }
            _ => Err(refuse_open(options.writable, options.disposition)),
        }
    }

    fn open_metadata_file(
        &self,
        callback_path: &str,
        metadata: VfsMeta,
        writable: bool,
    ) -> io::Result<HandleId> {
        match self.route(callback_path)? {
            Routed::Inside(child) => {
                let pin = self.ensure_child(&child)?;
                let inner = pin.engine.open_metadata_file(
                    &child.engine_path(self.dialect),
                    metadata,
                    writable,
                )?;
                Ok(self.add_handle(pin, inner))
            }
            _ => Err(refuse_open(writable, OpenDisposition::OpenExisting)),
        }
    }

    fn read(&self, handle: HandleId, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        self.with_handle(handle, |pin, inner| pin.engine.read(inner, offset, output))
    }

    fn write(&self, handle: HandleId, offset: u64, input: &[u8]) -> io::Result<usize> {
        self.with_handle(handle, |pin, inner| pin.engine.write(inner, offset, input))
    }

    fn append(&self, handle: HandleId, input: &[u8]) -> io::Result<usize> {
        self.with_handle(handle, |pin, inner| pin.engine.append(inner, input))
    }

    fn len(&self, handle: HandleId) -> io::Result<u64> {
        self.with_handle(handle, |pin, inner| pin.engine.len(inner))
    }

    fn truncate(&self, handle: HandleId, length: u64) -> io::Result<()> {
        self.with_handle(handle, |pin, inner| pin.engine.truncate(inner, length))
    }

    fn flush(&self, handle: HandleId) -> io::Result<FlushOutcome> {
        self.with_handle(handle, |pin, inner| pin.engine.flush(inner))
    }

    fn close(&self, handle: HandleId) -> io::Result<()> {
        let (pin, inner) = guard(&self.handles)
            .remove(&handle.0)
            .ok_or_else(invalid_handle)?;
        pin.engine.close(inner)
    }

    fn set_pending_attributes(
        &self,
        handle: HandleId,
        attributes: PendingAttributes,
    ) -> io::Result<()> {
        self.with_handle(handle, |pin, inner| {
            pin.engine.set_pending_attributes(inner, attributes)
        })
    }

    fn stat(&self, callback_path: &str) -> io::Result<VfsMeta> {
        match self.route(callback_path)? {
            Routed::Inside(child) => self
                .ensure_child(&child)?
                .engine
                .stat(&child.engine_path(self.dialect)),
            _ => Ok(self.dir_meta(&self.leaf(callback_path))),
        }
    }

    fn stat_cached(&self, callback_path: &str) -> io::Result<VfsMeta> {
        match self.route(callback_path)? {
            Routed::Inside(child) => self
                .ensure_child(&child)?
                .engine
                .stat_cached(&child.engine_path(self.dialect)),
            _ => Ok(self.dir_meta(&self.leaf(callback_path))),
        }
    }

    fn stat_handle(&self, handle: HandleId) -> io::Result<VfsMeta> {
        self.with_handle(handle, |pin, inner| pin.engine.stat_handle(inner))
    }

    fn list_dir(&self, callback_path: &str) -> io::Result<Vec<VfsMeta>> {
        match self.route(callback_path)? {
            Routed::Virtual(dir) => self.list_virtual(&dir),
            Routed::Share(child) | Routed::Inside(child) => self
                .ensure_child(&child)?
                .engine
                .list_dir(&child.engine_path(self.dialect)),
        }
    }

    fn list_dir_cached(&self, callback_path: &str) -> io::Result<Arc<[VfsMeta]>> {
        match self.route(callback_path)? {
            Routed::Virtual(dir) => Ok(Arc::from(self.list_virtual(&dir)?)),
            Routed::Share(child) | Routed::Inside(child) => self
                .ensure_child(&child)?
                .engine
                .list_dir_cached(&child.engine_path(self.dialect)),
        }
    }

    fn mkdir(&self, callback_path: &str) -> io::Result<NamespaceOutcome> {
        match self.route_new(callback_path)? {
            Routed::Inside(child) => self
                .ensure_child(&child)?
                .engine
                .mkdir(&child.engine_path(self.dialect)),
            _ => Err(denied()),
        }
    }

    fn rename(
        &self,
        source_callback_path: &str,
        destination_callback_path: &str,
        replace_existing: bool,
    ) -> io::Result<RenameOutcome> {
        self.within_one_child(
            source_callback_path,
            destination_callback_path,
            |pin, from, to| pin.engine.rename(from, to, replace_existing),
        )
    }

    fn rename_with_shared_destination(
        &self,
        source_callback_path: &str,
        destination_callback_path: &str,
        replace_existing: bool,
        shared_destination_is_open: bool,
    ) -> io::Result<RenameOutcome> {
        self.within_one_child(
            source_callback_path,
            destination_callback_path,
            |pin, from, to| {
                pin.engine.rename_with_shared_destination(
                    from,
                    to,
                    replace_existing,
                    shared_destination_is_open,
                )
            },
        )
    }

    fn flush_path(&self, callback_path: &str) -> io::Result<FlushOutcome> {
        match self.route(callback_path)? {
            Routed::Inside(child) => match self.pin_open(&child.child) {
                Some(pin) => pin.engine.flush_path(&child.engine_path(self.dialect)),
                None => Ok(FlushOutcome::NoChanges),
            },
            _ => Ok(FlushOutcome::NoChanges),
        }
    }

    fn begin_delete(&self, callback_path: &str, is_directory: bool) -> io::Result<DeleteToken> {
        match self.route(callback_path)? {
            Routed::Inside(child) => {
                let pin = self.ensure_child(&child)?;
                let inner = pin
                    .engine
                    .begin_delete(&child.engine_path(self.dialect), is_directory)?;
                let id = self.next_delete.fetch_add(1, Ordering::Relaxed);
                guard(&self.deletes).insert(id, (pin, inner));
                Ok(DeleteToken(id))
            }
            _ => Err(denied()),
        }
    }

    fn cancel_delete(&self, token: DeleteToken) -> io::Result<()> {
        let (pin, inner) = guard(&self.deletes)
            .remove(&token.0)
            .ok_or_else(invalid_handle)?;
        pin.engine.cancel_delete(inner)
    }

    fn commit_delete(&self, token: DeleteToken) -> io::Result<()> {
        let (pin, inner) = guard(&self.deletes)
            .remove(&token.0)
            .ok_or_else(invalid_handle)?;
        pin.engine.commit_delete(inner)
    }

    fn delete(&self, callback_path: &str, is_directory: bool) -> io::Result<()> {
        match self.route(callback_path)? {
            Routed::Inside(child) => self
                .ensure_child(&child)?
                .engine
                .delete(&child.engine_path(self.dialect), is_directory),
            _ => Err(denied()),
        }
    }

    fn pending_deletes(&self) -> io::Result<Vec<DeleteToken>> {
        Ok(guard(&self.deletes)
            .keys()
            .map(|id| DeleteToken(*id))
            .collect())
    }

    fn retry_pending_changes(&self) -> io::Result<()> {
        let mut first = None;
        for (key, child) in self.open_children() {
            if self.online(&key.key) && child.pending() > 0 {
                if let Err(error) = child.retry() {
                    first.get_or_insert(error);
                }
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn dirty_entries(&self) -> io::Result<Vec<(String, EntryCondition)>> {
        let mut all = Vec::new();
        for (key, child) in self.open_children() {
            let prefix = self
                .prefix_of(&key)
                .unwrap_or_else(|| format!("/{}/{}", key.key, key.name));
            for (path, condition) in child.engine.dirty_entries()? {
                all.push((Self::lift(&prefix, &path), condition));
            }
        }
        for (path, reason) in guard(&self.broken).iter() {
            let conflict = MountConflict {
                path: path.clone(),
                baseline: Baseline::Missing,
                current: None,
                detail: reason.clone(),
            };
            all.push((path.clone(), EntryCondition::Conflict(conflict)));
        }
        all.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(all)
    }

    fn maintain_cache(&self) -> io::Result<()> {
        let mut first = None;
        for (_, child) in self.open_children() {
            if let Err(error) = child.engine.maintain_cache() {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn evict_clean(&self, callback_path: &str) -> io::Result<bool> {
        match self.route(callback_path)? {
            Routed::Inside(child) => match self.pin_open(&child.child) {
                Some(pin) => pin.engine.evict_clean(&child.engine_path(self.dialect)),
                None => Ok(false),
            },
            _ => Ok(false),
        }
    }

    /// The device list needs no network.
    fn preload_metadata(&self) -> io::Result<()> {
        Ok(())
    }

    fn preload_metadata_while(&self, stopped: StopProbe<'_>) -> io::Result<PreloadProgress> {
        let mut total = PreloadProgress::default();
        for (key, child) in self.open_children() {
            if stopped() {
                break;
            }
            if !self.online(&key.key) {
                continue;
            }
            let step = MountFilesystem::preload_metadata_while(&*child.engine, stopped)?;
            total.loaded += step.loaded;
            total.pending |= step.pending;
        }
        Ok(total)
    }

    fn refresh_metadata_while(&self, stopped: StopProbe<'_>) -> io::Result<()> {
        for (key, child) in self.open_children() {
            if stopped() {
                return Ok(());
            }
            if self.online(&key.key) {
                let _ = MountFilesystem::refresh_metadata_while(&*child.engine, stopped);
            }
        }
        let browsed: Vec<String> = guard(&self.roots).keys().cloned().collect();
        for key in browsed {
            if stopped() {
                break;
            }
            if self.online(&key) {
                self.refresh_roots(&key);
            }
        }
        Ok(())
    }

    fn drain_metadata_changes(&self, limit: usize) -> io::Result<Vec<MetadataChange>> {
        let mut out = {
            let mut state = guard(&self.state);
            let take = limit.min(state.changes.len());
            state.changes.drain(..take).collect::<Vec<_>>()
        };
        for (key, child) in self.open_children() {
            if out.len() >= limit {
                break;
            }
            let Some(prefix) = self.prefix_of(&key) else {
                continue;
            };
            for change in child.engine.drain_metadata_changes(limit - out.len())? {
                out.push(match change {
                    MetadataChange::Created { path, is_directory } => MetadataChange::Created {
                        path: Self::lift(&prefix, &path),
                        is_directory,
                    },
                    MetadataChange::Deleted { path, is_directory } => MetadataChange::Deleted {
                        path: Self::lift(&prefix, &path),
                        is_directory,
                    },
                    MetadataChange::Modified { path } => MetadataChange::Modified {
                        path: Self::lift(&prefix, &path),
                    },
                });
            }
        }
        Ok(out)
    }

    fn invalidate_content(&self, backend_path: &str, recursive: bool) {
        let segments: Vec<String> = backend_path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect();
        if let Ok(Routed::Inside(child)) = self.route_segments(&segments, false) {
            if let Some(pin) = self.pin_open(&child.child) {
                pin.engine
                    .invalidate_content(&child.backend_path(), recursive);
            }
        }
    }

    /// The device tree is no vault drive: nothing to publish for a lock.
    fn prepare_lock(&self) -> io::Result<LockReadiness> {
        Ok(LockReadiness::default())
    }
}
