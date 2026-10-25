//! The provider's consent boundary precedes both mount caches and backend I/O.
//! A failed check permanently quarantines this engine; rebinding the site must
//! never make its old handles usable again. No table or register lock is held.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::mount::{
    DeleteToken, EntryCondition, FlushOutcome, HandleId, LockReadiness, MetadataChange,
    MountFilesystem, NamespaceOutcome, OpenFileOptions, PendingAttributes, PreloadProgress,
    ProjectedPath, RenameOutcome, StopProbe, VolumeFacts,
};
use crate::vault::{MountAccess, VaultSession};
use crate::vfs::VfsMeta;

pub(super) struct Access {
    consent: MountAccess,
    session: Arc<VaultSession>,
    revoked: AtomicBool,
}

impl Access {
    pub(super) fn new(consent: MountAccess, session: Arc<VaultSession>) -> Self {
        Self { consent, session, revoked: AtomicBool::new(false) }
    }

    pub(super) fn revoke(&self) { self.revoked.store(true, Ordering::Release); }

    pub(super) fn check(&self) -> io::Result<()> {
        if !self.revoked.load(Ordering::Acquire) && !self.session.is_locked()
            && self.consent.check().is_ok() && !self.session.is_locked()
            && !self.revoked.load(Ordering::Acquire)
        {
            return Ok(());
        }
        self.revoke();
        Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "Die Speicherfreigabe dieses Tresors fehlt oder wurde widerrufen"))
    }
}

pub(super) fn protect(
    inner: Arc<dyn MountFilesystem>, access: Option<Arc<Access>>,
) -> Arc<dyn MountFilesystem> {
    match access {
        Some(access) => Arc::new(Protected { inner, access }),
        None => inner,
    }
}

struct Protected {
    inner: Arc<dyn MountFilesystem>,
    access: Arc<Access>,
}

// Keeping the check on the frontend side also covers a metadata-only handle,
// an already materialized spool and a no-op flush without backend traffic.
macro_rules! checked {
    ($(fn $name:ident(&self $(, $arg:ident: $ty:ty)*) -> $out:ty;)+) => {
        $(fn $name(&self $(, $arg: $ty)*) -> $out {
            self.access.check()?;
            self.inner.$name($($arg),*)
        })+
    };
}

impl MountFilesystem for Protected {
    fn capacity_snapshot(&self) -> crate::vfs::CapacitySnapshot {
        if self.access.check().is_ok() { return self.inner.capacity_snapshot(); }
        let facts = self.inner.volume_facts();
        crate::vfs::CapacitySnapshot::unknown(
            format!("mount:{}", facts.id.as_str()), facts.root.as_str())
    }

    fn cache_capacity_budget(&self, available_bytes: u64) -> u64 {
        if self.access.check().is_ok() { self.inner.cache_capacity_budget(available_bytes) }
        else { 0 }
    }

    fn volume_facts(&self) -> VolumeFacts {
        let mut facts = self.inner.volume_facts();
        if self.access.check().is_err() { facts.read_only = true; }
        facts
    }

    checked! {
        fn project(&self, callback_path: &str) -> io::Result<ProjectedPath>;
        fn open_file(&self, callback_path: &str, options: OpenFileOptions) -> io::Result<HandleId>;
        fn open_metadata_file(&self, callback_path: &str, metadata: VfsMeta, writable: bool) -> io::Result<HandleId>;
        fn read(&self, handle: HandleId, offset: u64, output: &mut [u8]) -> io::Result<usize>;
        fn write(&self, handle: HandleId, offset: u64, input: &[u8]) -> io::Result<usize>;
        fn append(&self, handle: HandleId, input: &[u8]) -> io::Result<usize>;
        fn len(&self, handle: HandleId) -> io::Result<u64>;
        fn truncate(&self, handle: HandleId, length: u64) -> io::Result<()>;
        fn flush(&self, handle: HandleId) -> io::Result<FlushOutcome>;
        fn set_pending_attributes(&self, handle: HandleId, attributes: PendingAttributes) -> io::Result<()>;
        fn stat(&self, callback_path: &str) -> io::Result<VfsMeta>;
        fn stat_cached(&self, callback_path: &str) -> io::Result<VfsMeta>;
        fn stat_handle(&self, handle: HandleId) -> io::Result<VfsMeta>;
        fn list_dir(&self, callback_path: &str) -> io::Result<Vec<VfsMeta>>;
        fn list_dir_cached(&self, callback_path: &str) -> io::Result<Arc<[VfsMeta]>>;
        fn mkdir(&self, callback_path: &str) -> io::Result<NamespaceOutcome>;
        fn rename(&self, source_callback_path: &str, destination_callback_path: &str, replace_existing: bool) -> io::Result<RenameOutcome>;
        fn rename_with_shared_destination(&self, source_callback_path: &str, destination_callback_path: &str, replace_existing: bool, shared_destination_is_open: bool) -> io::Result<RenameOutcome>;
        fn flush_path(&self, callback_path: &str) -> io::Result<FlushOutcome>;
        fn begin_delete(&self, callback_path: &str, is_directory: bool) -> io::Result<DeleteToken>;
        fn cancel_delete(&self, token: DeleteToken) -> io::Result<()>;
        fn commit_delete(&self, token: DeleteToken) -> io::Result<()>;
        fn delete(&self, callback_path: &str, is_directory: bool) -> io::Result<()>;
        fn pending_deletes(&self) -> io::Result<Vec<DeleteToken>>;
        fn retry_pending_changes(&self) -> io::Result<()>;
        fn dirty_entries(&self) -> io::Result<Vec<(String, EntryCondition)>>;
        fn maintain_cache(&self) -> io::Result<()>;
        fn evict_clean(&self, callback_path: &str) -> io::Result<bool>;
        fn preload_metadata(&self) -> io::Result<()>;
        fn preload_metadata_while(&self, stopped: StopProbe<'_>) -> io::Result<PreloadProgress>;
        fn refresh_metadata_while(&self, stopped: StopProbe<'_>) -> io::Result<()>;
        fn drain_metadata_changes(&self, limit: usize) -> io::Result<Vec<MetadataChange>>;
        fn prepare_lock(&self) -> io::Result<LockReadiness>;
    }

    fn close(&self, handle: HandleId) -> io::Result<()> {
        let allowed = self.access.check();
        // Close never publishes; it releases the pin, preserving every dirty
        // journal entry even after consent was revoked. Do not call flush here.
        let closed = self.inner.close(handle);
        allowed.and(closed)
    }

    fn invalidate_content(&self, backend_path: &str, recursive: bool) {
        if self.access.check().is_ok() {
            self.inner.invalidate_content(backend_path, recursive);
        }
    }
}
