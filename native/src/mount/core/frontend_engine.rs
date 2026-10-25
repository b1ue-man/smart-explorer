//! `MountEngine` as `MountFilesystem` (DRV-M2). Every operation delegates
//! to the engine method of the same name, so a frontend running over the
//! trait behaves exactly like one calling the engine directly.
use std::io;
use std::sync::Arc;

use crate::vfs::VfsMeta;

use super::engine::MountEngine;
use super::frontend::{
    LockReadiness, MountFilesystem, PendingAttributes, PreloadProgress, StopProbe, VolumeFacts,
};
use super::metadata_cache::MetadataChange;
use super::path::ProjectedPath;
use super::types::{
    DeleteToken, EntryCondition, FlushOutcome, HandleId, MountMode, NamespaceOutcome,
    OpenFileOptions, RenameOutcome,
};

impl MountFilesystem for MountEngine {
    fn capacity_snapshot(&self) -> crate::vfs::CapacitySnapshot { MountEngine::capacity_snapshot(self) }
    fn cache_capacity_budget(&self, available_bytes: u64) -> u64 { MountEngine::cache_capacity_budget(self, available_bytes) }
    fn volume_facts(&self) -> VolumeFacts {
        VolumeFacts {
            id: self.config.id.clone(),
            root: self.projector.root().clone(),
            dialect: self.projector.dialect(),
            case_sensitive: self.case_sensitive_paths,
            read_only: self.config.mode == MountMode::ReadOnly,
        }
    }

    fn project(&self, callback_path: &str) -> io::Result<ProjectedPath> {
        self.projector.project(callback_path)
    }

    fn open_file(&self, callback_path: &str, options: OpenFileOptions) -> io::Result<HandleId> {
        MountEngine::open_file(self, callback_path, options)
    }

    fn open_metadata_file(
        &self,
        callback_path: &str,
        metadata: VfsMeta,
        writable: bool,
    ) -> io::Result<HandleId> {
        MountEngine::open_metadata_file(self, callback_path, metadata, writable)
    }

    fn read(&self, handle: HandleId, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        MountEngine::read(self, handle, offset, output)
    }

    fn write(&self, handle: HandleId, offset: u64, input: &[u8]) -> io::Result<usize> {
        MountEngine::write(self, handle, offset, input)
    }

    fn append(&self, handle: HandleId, input: &[u8]) -> io::Result<usize> {
        MountEngine::append(self, handle, input)
    }

    fn len(&self, handle: HandleId) -> io::Result<u64> {
        MountEngine::len(self, handle)
    }

    fn truncate(&self, handle: HandleId, length: u64) -> io::Result<()> {
        MountEngine::truncate(self, handle, length)
    }

    fn flush(&self, handle: HandleId) -> io::Result<FlushOutcome> {
        MountEngine::flush(self, handle)
    }

    fn close(&self, handle: HandleId) -> io::Result<()> {
        MountEngine::close(self, handle)
    }

    fn set_pending_attributes(
        &self,
        handle: HandleId,
        attributes: PendingAttributes,
    ) -> io::Result<()> {
        // Windows never sets times or modes (`set_file_time` answers
        // ERROR_NOT_SUPPORTED); a drive of the Windows dialect refuses them,
        // a POSIX drive keeps them for the next publication (DRV-M4).
        MountEngine::set_pending_attributes(self, handle, attributes)
    }

    fn stat(&self, callback_path: &str) -> io::Result<VfsMeta> {
        MountEngine::stat(self, callback_path)
    }

    fn stat_cached(&self, callback_path: &str) -> io::Result<VfsMeta> {
        MountEngine::stat_cached(self, callback_path)
    }

    fn stat_handle(&self, handle: HandleId) -> io::Result<VfsMeta> {
        MountEngine::stat_handle(self, handle)
    }

    fn list_dir(&self, callback_path: &str) -> io::Result<Vec<VfsMeta>> {
        MountEngine::list_dir(self, callback_path)
    }

    fn list_dir_cached(&self, callback_path: &str) -> io::Result<Arc<[VfsMeta]>> {
        MountEngine::list_dir_cached(self, callback_path)
    }

    fn mkdir(&self, callback_path: &str) -> io::Result<NamespaceOutcome> {
        MountEngine::mkdir(self, callback_path)
    }

    fn rename(
        &self,
        source_callback_path: &str,
        destination_callback_path: &str,
        replace_existing: bool,
    ) -> io::Result<RenameOutcome> {
        MountEngine::rename(
            self,
            source_callback_path,
            destination_callback_path,
            replace_existing,
        )
    }

    fn rename_with_shared_destination(
        &self,
        source_callback_path: &str,
        destination_callback_path: &str,
        replace_existing: bool,
        shared_destination_is_open: bool,
    ) -> io::Result<RenameOutcome> {
        MountEngine::rename_with_shared_destination(
            self,
            source_callback_path,
            destination_callback_path,
            replace_existing,
            shared_destination_is_open,
        )
    }

    fn flush_path(&self, callback_path: &str) -> io::Result<FlushOutcome> {
        MountEngine::flush_path(self, callback_path)
    }

    fn begin_delete(&self, callback_path: &str, is_directory: bool) -> io::Result<DeleteToken> {
        MountEngine::begin_delete(self, callback_path, is_directory)
    }

    fn cancel_delete(&self, token: DeleteToken) -> io::Result<()> {
        MountEngine::cancel_delete(self, token)
    }

    fn commit_delete(&self, token: DeleteToken) -> io::Result<()> {
        MountEngine::commit_delete(self, token)
    }

    fn delete(&self, callback_path: &str, is_directory: bool) -> io::Result<()> {
        MountEngine::delete(self, callback_path, is_directory)
    }

    fn pending_deletes(&self) -> io::Result<Vec<DeleteToken>> {
        MountEngine::pending_deletes(self)
    }

    fn retry_pending_changes(&self) -> io::Result<()> {
        MountEngine::retry_pending_changes(self)
    }

    fn dirty_entries(&self) -> io::Result<Vec<(String, EntryCondition)>> {
        MountEngine::dirty_entries(self)
    }

    fn maintain_cache(&self) -> io::Result<()> {
        MountEngine::maintain_cache(self)
    }

    fn evict_clean(&self, callback_path: &str) -> io::Result<bool> {
        MountEngine::evict_clean(self, callback_path)
    }

    fn preload_metadata(&self) -> io::Result<()> {
        MountEngine::preload_metadata(self)
    }

    fn preload_metadata_while(&self, stopped: StopProbe<'_>) -> io::Result<PreloadProgress> {
        self.preload_metadata_progress_while(stopped)
            .map(|progress| PreloadProgress {
                loaded: progress.loaded,
                pending: progress.pending,
            })
    }

    fn refresh_metadata_while(&self, stopped: StopProbe<'_>) -> io::Result<()> {
        MountEngine::refresh_metadata_while(self, stopped)
    }

    fn drain_metadata_changes(&self, limit: usize) -> io::Result<Vec<MetadataChange>> {
        MountEngine::drain_metadata_changes(self, limit)
    }

    fn invalidate_content(&self, backend_path: &str, recursive: bool) {
        MountEngine::invalidate_content(self, backend_path, recursive)
    }

    fn prepare_lock(&self) -> io::Result<LockReadiness> {
        // Publishing failures are reported, not fatal: the lock participant
        // decides with its deadline whether sealed changes stay behind.
        let published = MountEngine::retry_pending_changes(self);
        MountEngine::maintain_cache(self)?;
        self.clean_cache.trim(&self.spool, 0)?;
        Ok(LockReadiness {
            pending: MountEngine::dirty_entries(self)?.len(),
            detail: published.err().map(|error| error.to_string()),
        })
    }
}
