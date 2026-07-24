use super::rooted_backend::RootedBackend;
use crate::mount::{BackendRoot, MountMode, MountRootSecurity};
use crate::vfs::{
    Backend, BackendHandle, LocalBackend, MountPathCapabilities, RootConfinement, Scheme,
    StagedWriteCapabilities, VfsMeta, VfsResult,
};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct RootConfinedLocalBackend {
    inner: LocalBackend,
    scheme: Scheme,
    mount_capability_probes: Arc<AtomicUsize>,
    stat_calls: Arc<AtomicUsize>,
    list_calls: Arc<AtomicUsize>,
    case_sensitive: bool,
    root_confinement: RootConfinement,
}

impl Backend for RootConfinedLocalBackend {
    fn scheme(&self) -> Scheme {
        self.scheme
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.stat_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.stat(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }

    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write_new(path)
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename(source, destination)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }

    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        panic!("mount setup must use the combined capability snapshot")
    }

    fn case_sensitive_paths(&self, _root: &str) -> bool {
        self.case_sensitive
    }

    fn root_confinement(&self, _root: &str) -> RootConfinement {
        panic!("mount setup must use the combined capability snapshot")
    }

    fn mount_path_capabilities(&self, _root: &str) -> VfsResult<MountPathCapabilities> {
        self.mount_capability_probes.fetch_add(1, Ordering::SeqCst);
        Ok(MountPathCapabilities {
            staged_write: StagedWriteCapabilities::complete(),
            root_confinement: self.root_confinement,
        })
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    root_path: std::path::PathBuf,
    root: BackendRoot,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root_path = directory.path().join("selected-root");
        std::fs::create_dir(&root_path).unwrap();
        std::fs::write(root_path.join("note.md"), b"trusted contents").unwrap();
        let root = BackendRoot::parse(&forward_slashes(&root_path)).unwrap();
        Self {
            _directory: directory,
            root_path,
            root,
        }
    }

    fn unverified(&self) -> BackendHandle {
        Arc::new(LocalBackend::new(self.root.as_str()))
    }

    fn confined(&self) -> BackendHandle {
        Arc::new(RootConfinedLocalBackend {
            inner: LocalBackend::new(self.root.as_str()),
            scheme: Scheme::Sftp,
            mount_capability_probes: Arc::new(AtomicUsize::new(0)),
            stat_calls: Arc::new(AtomicUsize::new(0)),
            list_calls: Arc::new(AtomicUsize::new(0)),
            case_sensitive: true,
            root_confinement: RootConfinement::Enforced,
        })
    }
}

#[test]
fn remote_drive_task_enforced_root_rejects_unverified_backend() {
    let fixture = Fixture::new();
    let error = only_error(RootedBackend::new(
        fixture.unverified(),
        &fixture.root,
        MountMode::ReadOnly,
        MountRootSecurity::Enforced,
    ));
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn remote_drive_task_trusted_root_accepts_unverified_backend() {
    let fixture = Fixture::new();
    let backend = RootedBackend::new(
        fixture.unverified(),
        &fixture.root,
        MountMode::ReadOnly,
        MountRootSecurity::Trusted,
    )
    .unwrap();

    let root = backend.stat("/").unwrap();
    assert!(root.is_dir);
    let mut contents = String::new();
    backend
        .open_read("/note.md")
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents, "trusted contents");
}

#[test]
fn remote_drive_task_rooted_backend_consumes_one_combined_capability_snapshot() {
    let fixture = Fixture::new();
    let probes = Arc::new(AtomicUsize::new(0));
    let inner: BackendHandle = Arc::new(RootConfinedLocalBackend {
        inner: LocalBackend::new(fixture.root.as_str()),
        scheme: Scheme::Sftp,
        mount_capability_probes: probes.clone(),
        stat_calls: Arc::new(AtomicUsize::new(0)),
        list_calls: Arc::new(AtomicUsize::new(0)),
        case_sensitive: true,
        root_confinement: RootConfinement::Enforced,
    });

    let backend = RootedBackend::new(
        inner,
        &fixture.root,
        MountMode::ReadWrite,
        MountRootSecurity::Enforced,
    )
    .unwrap();

    assert_eq!(probes.load(Ordering::SeqCst), 1);
    assert_eq!(
        backend.mount_path_capabilities("/").unwrap(),
        MountPathCapabilities {
            staged_write: StagedWriteCapabilities::complete(),
            root_confinement: RootConfinement::Enforced,
        }
    );
    assert_eq!(probes.load(Ordering::SeqCst), 1);
}

#[test]
fn remote_drive_task_enforced_reads_reuse_resolution_but_keep_final_stat_live() {
    let fixture = Fixture::new();
    let stats = Arc::new(AtomicUsize::new(0));
    let lists = Arc::new(AtomicUsize::new(0));
    let inner: BackendHandle = Arc::new(RootConfinedLocalBackend {
        inner: LocalBackend::new(fixture.root.as_str()),
        scheme: Scheme::Sftp,
        mount_capability_probes: Arc::new(AtomicUsize::new(0)),
        stat_calls: stats.clone(),
        list_calls: lists.clone(),
        case_sensitive: false,
        root_confinement: RootConfinement::Enforced,
    });
    let backend = RootedBackend::new(
        inner,
        &fixture.root,
        MountMode::ReadOnly,
        MountRootSecurity::Enforced,
    )
    .unwrap();
    let stats_before_reads = stats.load(Ordering::SeqCst);
    let lists_before_reads = lists.load(Ordering::SeqCst);

    let metadata = backend.stat("/NOTE.MD").unwrap();
    std::fs::write(
        fixture.root_path.join("note.md"),
        b"externally changed and longer contents",
    )
    .unwrap();
    let repeated = backend.stat("/note.md").unwrap();

    assert_eq!(metadata.name, "note.md");
    assert_eq!(repeated.name, "note.md");
    assert_ne!(
        metadata.size, repeated.size,
        "final stat must bypass the cache"
    );
    assert_eq!(
        stats.load(Ordering::SeqCst) - stats_before_reads,
        2,
        "each final stat remains a live backend observation"
    );
    assert_eq!(
        lists.load(Ordering::SeqCst) - lists_before_reads,
        1,
        "case resolution and final stat reuse one bounded parent listing"
    );
}

#[test]
fn remote_drive_task_trusted_reads_cache_case_resolution_but_revalidate_root() {
    let fixture = Fixture::new();
    let stats = Arc::new(AtomicUsize::new(0));
    let lists = Arc::new(AtomicUsize::new(0));
    let inner: BackendHandle = Arc::new(RootConfinedLocalBackend {
        inner: LocalBackend::new(fixture.root.as_str()),
        scheme: Scheme::Peer,
        mount_capability_probes: Arc::new(AtomicUsize::new(0)),
        stat_calls: stats.clone(),
        list_calls: lists.clone(),
        case_sensitive: false,
        root_confinement: RootConfinement::Unverified,
    });
    let backend = RootedBackend::new(
        inner,
        &fixture.root,
        MountMode::ReadOnly,
        MountRootSecurity::Trusted,
    )
    .unwrap();
    let stats_before_reads = stats.load(Ordering::SeqCst);
    let lists_before_reads = lists.load(Ordering::SeqCst);

    assert_eq!(backend.stat("/NOTE.MD").unwrap().name, "note.md");
    assert_eq!(backend.stat("/note.md").unwrap().name, "note.md");

    assert_eq!(
        stats.load(Ordering::SeqCst) - stats_before_reads,
        4,
        "trusted reads revalidate the root and observe the final target live"
    );
    assert_eq!(
        lists.load(Ordering::SeqCst) - lists_before_reads,
        1,
        "trusted case resolution must reuse the shared listing cache"
    );
}

#[test]
fn remote_drive_task_confined_projection_blocks_escape_and_preserves_exclusive_owner() {
    let fixture = Fixture::new();
    let outside = fixture._directory.path().join("outside.txt");
    std::fs::write(&outside, b"outside owner").unwrap();
    std::fs::write(fixture.root_path.join("claimed.stage"), b"first owner").unwrap();
    std::os::unix::fs::symlink(&outside, fixture.root_path.join("escape-link")).unwrap();

    let backend = RootedBackend::new(
        fixture.confined(),
        &fixture.root,
        MountMode::ReadWrite,
        MountRootSecurity::Enforced,
    )
    .unwrap();

    let traversal = only_error(backend.open_read("/../outside.txt"));
    assert_eq!(traversal.kind(), io::ErrorKind::InvalidInput);
    let linked = only_error(backend.open_read("/escape-link"));
    assert_eq!(linked.kind(), io::ErrorKind::PermissionDenied);

    let collision = only_error(backend.open_write_new("/claimed.stage"));
    assert_eq!(collision.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read(fixture.root_path.join("claimed.stage")).unwrap(),
        b"first owner"
    );
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside owner");

    let mut fresh = backend.open_write_new("/fresh.stage").unwrap();
    fresh.write_all(b"new owner").unwrap();
    fresh.flush().unwrap();
    drop(fresh);
    assert_eq!(
        std::fs::read(fixture.root_path.join("fresh.stage")).unwrap(),
        b"new owner"
    );

    assert!(!backend
        .list_dir("/")
        .unwrap()
        .iter()
        .any(|entry| entry.name == "nested"));
    backend.mkdir_all("/nested/child").unwrap();
    assert!(backend
        .list_dir("/")
        .unwrap()
        .iter()
        .any(|entry| entry.name == "nested"));
}

#[test]
fn remote_drive_task_case_colliding_listing_fails_closed() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root_path.join("NOTE.md"), b"collision").unwrap();
    let backend = RootedBackend::new(
        Arc::new(RootConfinedLocalBackend {
            inner: LocalBackend::new(fixture.root.as_str()),
            scheme: Scheme::Sftp,
            mount_capability_probes: Arc::new(AtomicUsize::new(0)),
            stat_calls: Arc::new(AtomicUsize::new(0)),
            list_calls: Arc::new(AtomicUsize::new(0)),
            case_sensitive: false,
            root_confinement: RootConfinement::Enforced,
        }),
        &fixture.root,
        MountMode::ReadOnly,
        MountRootSecurity::Enforced,
    )
    .unwrap();

    let error = backend.stat("/note.md").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

fn only_error<T>(result: io::Result<T>) -> io::Error {
    match result {
        Ok(_) => panic!("operation unexpectedly succeeded"),
        Err(error) => error,
    }
}

fn forward_slashes(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
