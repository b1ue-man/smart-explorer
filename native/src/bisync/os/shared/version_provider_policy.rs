//! Provider archive selection never derives permission from an error message.
use super::transfer_stream::check;
use super::version_manifest::invalid;
use super::versions::VersionSide;
use crate::vfs::{self, VersionArchivePolicy};
use std::io;
use std::sync::atomic::AtomicBool;

pub(super) fn uses_provider_archive(
    side: &VersionSide<'_>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    check(cancel)?;
    if vfs::version_archive_policy(side.backend) == VersionArchivePolicy::Provider {
        return Ok(true);
    }
    // The archive is host-private, not missing. Observe current export access
    // through the live sync boundary before selecting the private local copy.
    let root = vfs::sync_stat(side.backend, side.root)?;
    if !root.is_dir || root.is_symlink || root.special {
        return Err(invalid("version fallback root is not plain"));
    }
    check(cancel)?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bisync::{PairLock, PairSide, StateOwner, VersionsLocation};
    use crate::vfs::{Backend, BackendExtensions, CachingBackend, LocalBackend, Scheme, VfsMeta};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const PAIR: &str = "0123456789abcdef0123456789abcdef";

    struct ArchiveBackend {
        inner: LocalBackend,
        root: String,
        policy: Option<VersionArchivePolicy>,
        denied: Mutex<Option<(String, io::ErrorKind)>>,
        deny_read: AtomicBool,
        shape: AtomicUsize,
        stats: AtomicUsize,
        archive_calls: AtomicUsize,
        _dir: tempfile::TempDir,
    }

    impl ArchiveBackend {
        fn new(policy: Option<VersionArchivePolicy>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("root");
            std::fs::create_dir(&root).unwrap();
            let root = root.to_str().unwrap().replace('\\', "/");
            Self {
                inner: LocalBackend::new(dir.path().to_str().unwrap()),
                root,
                policy,
                denied: Mutex::new(None),
                deny_read: AtomicBool::new(false),
                shape: AtomicUsize::new(0),
                stats: AtomicUsize::new(0),
                archive_calls: AtomicUsize::new(0),
                _dir: dir,
            }
        }

        fn side(&self) -> VersionSide<'_> {
            VersionSide {
                side: PairSide::B,
                backend: self,
                root: &self.root,
            }
        }

        fn access(&self, path: &str) -> io::Result<()> {
            if path.starts_with(&format!("{}/.se-versions", self.root)) {
                self.archive_calls.fetch_add(1, Ordering::SeqCst);
            }
            if let Some((denied, kind)) = &*self.denied.lock().unwrap() {
                if denied == path {
                    return Err(io::Error::new(*kind, "Pfad ist nicht freigegeben"));
                }
            }
            Ok(())
        }
    }

    impl Backend for ArchiveBackend {
        fn scheme(&self) -> Scheme {
            Scheme::Peer
        }
        fn root_display(&self) -> String {
            self.root.clone()
        }
        fn extensions(&self) -> Option<&dyn BackendExtensions> {
            self.policy.map(|_| self as &dyn BackendExtensions)
        }
        fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
            self.access(path)?;
            self.inner.list_dir(path)
        }
        fn stat(&self, path: &str) -> io::Result<VfsMeta> {
            self.stats.fetch_add(1, Ordering::SeqCst);
            self.access(path)?;
            let mut meta = self.inner.stat(path)?;
            if path == self.root {
                match self.shape.load(Ordering::SeqCst) {
                    1 => meta.is_dir = false,
                    2 => meta.is_symlink = true,
                    3 => meta.special = true,
                    _ => {}
                }
            }
            Ok(meta)
        }
        fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
            self.access(path)?;
            if self.deny_read.load(Ordering::SeqCst) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "backup read denied",
                ));
            }
            self.inner.open_read(path)
        }
        fn open_write(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
            self.inner.open_write(path)
        }
        fn rename(&self, src: &str, dst: &str) -> io::Result<()> {
            self.inner.rename(src, dst)
        }
        fn remove_file(&self, path: &str) -> io::Result<()> {
            self.inner.remove_file(path)
        }
        fn remove_dir(&self, path: &str) -> io::Result<()> {
            self.inner.remove_dir(path)
        }
        fn mkdir_all(&self, path: &str) -> io::Result<()> {
            self.inner.mkdir_all(path)
        }
        fn root_confinement(&self, root: &str) -> crate::vfs::RootConfinement {
            self.inner.root_confinement(root)
        }
    }

    impl BackendExtensions for ArchiveBackend {
        fn version_archive_policy(&self) -> VersionArchivePolicy {
            self.policy.unwrap_or_default()
        }
    }

    #[test]
    fn sync_reliability_task_old_jobs_private_archive_revalidates_cached_root_and_revoked_access() {
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
            let backend = Arc::new(ArchiveBackend::new(Some(VersionArchivePolicy::AppPrivate)));
            let private = std::path::Path::new(&backend.root).join(".se-versions/host-only");
            std::fs::create_dir_all(private.parent().unwrap()).unwrap();
            std::fs::write(&private, b"host private bytes").unwrap();
            let cached = CachingBackend::new(backend.clone());
            let parent = std::path::Path::new(&backend.root).parent().unwrap();
            cached
                .list_dir(&parent.to_str().unwrap().replace('\\', "/"))
                .unwrap();
            assert!(cached.stat(&backend.root).unwrap().is_dir);
            let side = VersionSide {
                side: PairSide::B,
                backend: &cached,
                root: &backend.root,
            };
            let cancel = AtomicBool::new(false);
            assert!(super::super::version_listing::managed_sync_root(&side, PAIR, &cancel)
                .unwrap()
                .is_empty());
            assert_eq!(backend.archive_calls.load(Ordering::SeqCst), 0);
            *backend.denied.lock().unwrap() = Some((backend.root.clone(), kind));
            // Browsing still has the old parent listing; versions must bypass it.
            assert!(cached.stat(&backend.root).unwrap().is_dir);
            let before = backend.stats.load(Ordering::SeqCst);
            let error = super::super::version_listing::managed_sync_root(&side, PAIR, &cancel)
                .err()
                .unwrap();
            assert_eq!(error.kind(), kind);
            assert!(backend.stats.load(Ordering::SeqCst) > before);
            assert_eq!(backend.archive_calls.load(Ordering::SeqCst), 0);
            assert_eq!(std::fs::read(private).unwrap(), b"host private bytes");
        }
    }

    #[test]
    fn sync_reliability_task_old_jobs_private_archive_rejects_cancelled_and_nonplain_roots() {
        let backend = ArchiveBackend::new(Some(VersionArchivePolicy::AppPrivate));
        let cancel = AtomicBool::new(true);
        assert!(uses_provider_archive(&backend.side(), &cancel).is_err());
        assert_eq!(backend.stats.load(Ordering::SeqCst), 0);
        cancel.store(false, Ordering::SeqCst);
        for shape in 1..=3 {
            backend.shape.store(shape, Ordering::SeqCst);
            assert_eq!(
                uses_provider_archive(&backend.side(), &cancel).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        assert_eq!(backend.archive_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn sync_reliability_task_old_jobs_unknown_provider_archive_and_denied_children_remain_errors() {
        let backend = ArchiveBackend::new(None);
        let root = format!("{}/.se-versions", backend.root);
        let child = format!("{root}/owned-run/side/item");
        std::fs::create_dir_all(&child).unwrap();
        let foreign = std::path::Path::new(&child).join("data");
        std::fs::write(&foreign, b"foreign archived bytes").unwrap();
        let cancel = AtomicBool::new(false);
        for denied in [&root, &format!("{root}/owned-run")] {
            *backend.denied.lock().unwrap() =
                Some((denied.clone(), io::ErrorKind::PermissionDenied));
            let error = super::super::version_listing::managed_sync_root(
                &backend.side(), PAIR, &cancel)
                .err()
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign archived bytes");
        }
        assert!(backend.archive_calls.load(Ordering::SeqCst) > 0);
    }

    #[test]
    fn sync_reliability_task_old_jobs_private_backup_read_failure_preserves_original_bytes() {
        use super::super::apply_guard::{capture, ExpectedFile};
        use super::super::versions::{RunVersions, VersionReason, VersionsContext};
        let backend = ArchiveBackend::new(Some(VersionArchivePolicy::AppPrivate));
        let app_data = tempfile::tempdir().unwrap();
        let source = format!("{}/file.txt", backend.root);
        std::fs::write(&source, b"original must survive").unwrap();
        let pair = crate::bisync::pair_id_for(&backend, &backend.root, &backend, &backend.root);
        let context = VersionsContext::new(
            &pair,
            StateOwner::AdHoc,
            VersionsLocation::Auto,
            Default::default(),
        );
        let versions = RunVersions::with_app_data(context, app_data.path().to_path_buf());
        let lock = PairLock::acquire(&pair).unwrap();
        versions.bind_lock(lock.id()).unwrap();
        let captured = capture(&backend, &source, ExpectedFile::Unknown, "backup failure").unwrap();
        backend.deny_read.store(true, Ordering::SeqCst);
        let error = super::super::version_save::save(
            &versions,
            &backend.side(),
            &source,
            "file.txt",
            &captured,
            ExpectedFile::Unknown,
            VersionReason::Replaced,
            &AtomicBool::new(false),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(&source).unwrap(), b"original must survive");
        assert_eq!(backend.archive_calls.load(Ordering::SeqCst), 0);
        assert!(super::super::version_listing::managed(
            &LocalBackend::new(app_data.path().to_str().unwrap()),
            app_data.path().to_str().unwrap(),
            &pair,
            super::super::versions::VersionStore::AppData,
            &AtomicBool::new(false),
        )
        .unwrap()
        .is_empty());
    }
}
