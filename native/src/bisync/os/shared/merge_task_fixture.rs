//! Remote-suite fixture for recorded merge publication failures.
use super::test_remote::FakeRemote;
use super::*;
use crate::vfs::{Backend, BackendExtensions, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
pub(super) fn signature(backend: &dyn Backend, path: &str) -> Sig {
    let meta = backend.stat(path).unwrap();
    Sig {
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        hash: 0,
    }
}
pub(super) fn state(a: &dyn Backend, ra: &str, b: &dyn Backend, rb: &str) -> StateKey {
    super::replica::identify(
        super::incremental::SyncEndpoints::new(a, ra, b, rb),
        &super::run_types::RunSettings::default(),
        false,
    )
    .unwrap()
    .key
}
pub(super) fn baseline(key: &StateKey) -> Baseline {
    let keys = super::KeyPolicy::default();
    super::checkpoint_journal::Journal::load(key, keys)
        .unwrap()
        .1
        .baseline
}
pub(super) fn seed(key: &StateKey, conflict: &Conflict) {
    let lock = PairLock::acquire(&key.lock_id).unwrap();
    super::replica_state::merge_baseline_entries(
        &lock,
        key,
        &[(conflict.rel.clone(), (conflict.a, conflict.b))],
    )
    .unwrap();
}
pub(super) fn clean(key: &StateKey) {
    let _ = std::fs::remove_dir_all(super::replica_state::pair_dir(&key.pair_id));
    let _ = std::fs::remove_dir_all(super::persistence::versions_dir(&key.pair_id));
}

/// Only publication of the second merged stage fails, after both backups
/// and the first durable publication; the same endpoint succeeds on retry.
pub(super) struct PublishOnce {
    pub inner: FakeRemote,
    pub fail: AtomicBool,
}
impl Backend for PublishOnce {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn state_identity(&self) -> String {
        self.inner.state_identity()
    }
    fn namespace_identity(&self) -> String {
        self.inner.namespace_identity()
    }
    fn list_dir(&self, p: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(p)
    }
    fn stat(&self, p: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(p)
    }
    fn open_read(&self, p: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(p)
    }
    fn open_write(&self, p: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(p)
    }
    fn open_write_new(&self, p: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write_new(p)
    }
    fn open_write_copy_stage(&self, p: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write_copy_stage(p)
    }
    fn open_write_copy_stage_sized(&self, p: &str, size: u64) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write_copy_stage_sized(p, size)
    }
    fn discard_copy_stage(&self, p: &str) -> VfsResult<()> {
        self.inner.discard_copy_stage(p)
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(&self.inner)
    }
    fn rename(&self, a: &str, b: &str) -> VfsResult<()> {
        self.inner.rename(a, b)
    }
    fn rename_no_replace(&self, a: &str, b: &str) -> VfsResult<()> {
        if crate::vfs::is_staging_name(a.rsplit('/').next().unwrap_or(a))
            && b.ends_with("/f.txt")
            && self.fail.swap(false, Ordering::SeqCst)
        {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "injected second-side publication failure",
            ));
        }
        self.inner.rename_no_replace(a, b)
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn remove_file(&self, p: &str) -> VfsResult<()> {
        self.inner.remove_file(p)
    }
    fn remove_dir(&self, p: &str) -> VfsResult<()> {
        self.inner.remove_dir(p)
    }
    fn mkdir_all(&self, p: &str) -> VfsResult<()> {
        self.inner.mkdir_all(p)
    }
}
