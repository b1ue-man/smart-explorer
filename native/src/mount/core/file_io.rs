use super::engine::{lock, read_lock, require_regular, write_lock, Entry, EntryState,
    MountEngine, OpenHandle, OpenHandleKind};
use super::entry_lifecycle::EntryPin;
use super::types::{EntryCondition, FlushOutcome, HandleId, MountMode, OpenDisposition, OpenFileOptions};
use crate::vfs::VfsMeta;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, atomic::Ordering};

impl MountEngine {
    pub fn open_file(&self, callback_path: &str, options: OpenFileOptions) -> io::Result<HandleId> {
        let _reap = self.operation_reaper();
        let creates_or_truncates = matches!(
            options.disposition,
            OpenDisposition::OpenOrCreate
                | OpenDisposition::CreateNew
                | OpenDisposition::TruncateExisting
                | OpenDisposition::CreateAlways
        );
        if (options.writable || creates_or_truncates) && self.config.mode != MountMode::ReadWrite {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mount is read-only",
            ));
        }
        let truncates = matches!(
            options.disposition,
            OpenDisposition::TruncateExisting | OpenDisposition::CreateAlways
        );
        let entry = self.materialize_at(callback_path, options.disposition)?;
        if truncates {
            self.truncate_entry(&entry, 0)?;
        }
        self.insert_handle(OpenHandleKind::Materialized(entry), options.writable)
    }

    /// Opens a regular file without fetching its contents into the whole-file
    /// spool. The first data operation upgrades the handle in place, so pure
    /// metadata and namespace traffic never occupies a backend transfer slot.
    pub(crate) fn open_metadata_file(
        &self,
        callback_path: &str,
        metadata: VfsMeta,
        writable: bool,
    ) -> io::Result<HandleId> {
        require_regular(&metadata)?;
        let _namespace = read_lock(&self.namespace)?;
        let path = self.projector.project(callback_path)?;
        self.validate_projected_case(&path)?;
        if path.relative().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mount root is not a regular file",
            ));
        }
        self.insert_handle(
            OpenHandleKind::Metadata {
                callback_path: callback_path.to_string(),
                meta: metadata,
            },
            writable,
        )
    }

    /// Returns the materialized entry for a handle, fetching the remote file
    /// on first data access of a lazily opened handle.
    fn ensure_materialized(&self, handle: HandleId) -> io::Result<EntryPin> {
        self.materialize_opened(handle, self.handle(handle)?)
    }

    fn materialize_opened(&self, handle: HandleId, opened: OpenHandle) -> io::Result<EntryPin> {
        let callback_path = match opened.kind {
            OpenHandleKind::Materialized(entry) => return Ok(entry),
            OpenHandleKind::Metadata { callback_path, .. } => callback_path,
        };
        let entry = self.materialize_at(&callback_path, OpenDisposition::OpenExisting)?;
        let mut handles = lock(&self.handles)?;
        match handles.get_mut(&handle) {
            Some(current) => match &current.kind {
                // Another data operation upgraded the handle while this one
                // was fetching; both raced on the same path guard, so they
                // resolved to the same cached entry.
                OpenHandleKind::Materialized(existing) => Ok(existing.clone()),
                OpenHandleKind::Metadata { .. } => {
                    current.kind = OpenHandleKind::Materialized(entry.clone());
                    Ok(entry)
                }
            },
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "mount handle closed during its first data access",
            )),
        }
    }

    pub fn read(&self, handle: HandleId, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        let _reap = self.operation_reaper();
        let entry = self.ensure_materialized(handle)?;
        let state = lock(&entry.state)?;
        let mut file = self.spool.open_file(&state.spool_name, false)?;
        file.seek(SeekFrom::Start(offset))?;
        // Windows callers treat a short count as end-of-file, so fill the
        // buffer until the spool is exhausted rather than returning one
        // arbitrary partial read.
        let mut total = 0;
        while total < output.len() {
            match file.read(&mut output[total..]) {
                Ok(0) => break,
                Ok(read) => total += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(total)
    }

    pub fn write(&self, handle: HandleId, offset: u64, input: &[u8]) -> io::Result<usize> {
        let _reap = self.operation_reaper();
        let opened = self.handle(handle)?;
        if !opened.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle is read-only",
            ));
        }
        let entry = self.materialize_opened(handle, opened)?;
        let mut state = lock(&entry.state)?;
        let mut file = self.spool.open_file(&state.spool_name, true)?;
        let end = offset.checked_add(input.len() as u64)
            .ok_or_else(|| io::Error::other("mounted write length overflow"))?;
        let _growth = self.reserve_growth(end.saturating_sub(file.metadata()?.len()))?;
        self.mark_dirty(&mut state)?;
        file.seek(SeekFrom::Start(offset))?;
        // A silently short write would be reported to Windows as success for
        // the smaller count and the remainder would never be retried.
        file.write_all(input)?;
        Ok(input.len())
    }

    pub fn append(&self, handle: HandleId, input: &[u8]) -> io::Result<usize> {
        let _reap = self.operation_reaper();
        let opened = self.handle(handle)?;
        if !opened.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle is read-only",
            ));
        }
        let entry = self.materialize_opened(handle, opened)?;
        let mut state = lock(&entry.state)?;
        let mut file = self.spool.open_file(&state.spool_name, true)?;
        let _growth = self.reserve_growth(input.len() as u64)?;
        self.mark_dirty(&mut state)?;
        file.seek(SeekFrom::End(0))?;
        file.write_all(input)?;
        Ok(input.len())
    }

    pub fn len(&self, handle: HandleId) -> io::Result<u64> {
        let _reap = self.operation_reaper();
        match self.handle(handle)?.kind {
            OpenHandleKind::Materialized(entry) => {
                let state = lock(&entry.state)?;
                Ok(self
                    .spool
                    .open_file(&state.spool_name, false)?
                    .metadata()?
                    .len())
            }
            // Length queries must not force a whole-file transfer.
            OpenHandleKind::Metadata { meta, .. } => Ok(meta.size),
        }
    }

    pub fn truncate(&self, handle: HandleId, length: u64) -> io::Result<()> {
        let _reap = self.operation_reaper();
        let opened = self.handle(handle)?;
        if !opened.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle is read-only",
            ));
        }
        let entry = self.materialize_opened(handle, opened)?;
        self.truncate_entry(&entry, length)
    }

    pub fn flush(&self, handle: HandleId) -> io::Result<FlushOutcome> {
        let _reap = self.operation_reaper();
        let opened = self.handle(handle)?;
        match opened.kind {
            OpenHandleKind::Materialized(entry) => self.flush_entry(&entry),
            OpenHandleKind::Metadata { .. } => Ok(FlushOutcome::NoChanges),
        }
    }

    pub fn close(&self, handle: HandleId) -> io::Result<()> {
        let _reap = self.operation_reaper();
        let opened = lock(&self.handles)?
            .remove(&handle)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown mount handle"))?;
        match opened.kind {
            OpenHandleKind::Materialized(entry) => {
                // Only materialized closes need the namespace write lock (for
                // spool cleanup racing renames). Metadata handles — the bulk
                // of Explorer traffic — must not queue a writer behind long
                // read-holding callbacks and stall the whole drive.
                let entry = entry.release();
                let _namespace = write_lock(&self.namespace)?;
                self.cleanup_committed_entry(&entry)
            }
            OpenHandleKind::Metadata { .. } => Ok(()),
        }
    }

    pub(super) fn handle(&self, handle: HandleId) -> io::Result<OpenHandle> {
        let handles = lock(&self.handles)?;
        handles
            .get(&handle)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown mount handle"))
    }

    fn insert_handle(&self, kind: OpenHandleKind, writable: bool) -> io::Result<HandleId> {
        let raw = self
            .next_handle
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| io::Error::other("mount handle space exhausted"))?;
        let handle = HandleId(raw);
        lock(&self.handles)?.insert(handle, OpenHandle { kind, writable });
        Ok(handle)
    }

    fn truncate_entry(&self, entry: &Arc<Entry>, length: u64) -> io::Result<()> {
        let mut state = lock(&entry.state)?;
        let file = self.spool.open_file(&state.spool_name, true)?;
        let _growth = self.reserve_growth(length.saturating_sub(file.metadata()?.len()))?;
        self.mark_dirty(&mut state)?;
        file.set_len(length)
    }

    fn mark_dirty(&self, state: &mut EntryState) -> io::Result<()> {
        if state.delete_committed {
            // Windows permits I/O through an already-open handle after its
            // name was replaced. Those bytes belong only to that detached
            // object and intentionally never re-enter the remote namespace.
            return Ok(());
        }
        match state.condition {
            EntryCondition::Clean => {
                let persisted = state.with_condition(EntryCondition::Dirty);
                if let Err(error) = self.spool.persist_entry(&persisted) {
                    state.condition = EntryCondition::Conflict(super::types::MountConflict {
                        path: state.remote_path.clone(), baseline: state.baseline.clone(), current: None,
                        detail: format!("dirty spool journal durability is uncertain: {error}"),
                    });
                    return Err(error);
                }
                state.condition = EntryCondition::Dirty;
                Ok(())
            }
            EntryCondition::Dirty => Ok(()),
            EntryCondition::Conflict(_) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "file has an unresolved remote conflict",
            )),
        }
    }

}
