use super::super::snapshot::{hash_mode, HashMode};
use super::super::*;
use super::{fwd, tmp};
use crate::vfs::{Backend, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

struct Native(LocalBackend);

impl Backend for Native {
    fn scheme(&self) -> Scheme {
        self.0.scheme()
    }
    fn root_display(&self) -> String {
        self.0.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.0.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.0.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.0.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.0.open_write(path)
    }
    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.0.rename(source, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.0.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.0.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir_all(path)
    }
    fn provides_content_hash(&self) -> bool {
        true
    }
}

#[test]
fn hash_mode_picks_cheapest_source() {
    let local = LocalBackend::new("/tmp");
    let native = Native(LocalBackend::new("/tmp"));
    assert_eq!(
        hash_mode(&native, &local, CompareMode::MtimeSize),
        HashMode::NativeOnly
    );
    assert_eq!(
        hash_mode(&local, &native, CompareMode::MtimeSize),
        HashMode::Full
    );
    assert_eq!(
        hash_mode(&local, &local, CompareMode::MtimeSize),
        HashMode::None
    );
    assert_eq!(
        hash_mode(&local, &native, CompareMode::SizeOnly),
        HashMode::None
    );
    assert_eq!(
        hash_mode(&local, &local, CompareMode::Checksum),
        HashMode::FullFresh
    );
}

#[test]
fn checksum_mode_never_treats_two_missing_hashes_as_equal() {
    let options = BisyncOptions {
        compare: CompareMode::Checksum,
        ..Default::default()
    };
    let signature = Sig {
        size: 4,
        mtime_ms: 7,
        hash: 0,
    };
    assert!(!super::super::core::sig_eq(
        Some(signature),
        Some(signature),
        &options
    ));
}

#[test]
fn walk_reuses_cached_hash_but_checksum_mode_is_fresh() {
    let directory = tmp("reuse");
    std::fs::write(directory.join("f.txt"), b"hello world").unwrap();
    let backend = LocalBackend::new(&fwd(&directory));
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let first = walk_files(
        &backend,
        &fwd(&directory),
        &cancel,
        &filter,
        HashMode::Full,
        None,
    )
    .unwrap();
    let real = first["f.txt"].hash;
    let metadata = backend.stat(&format!("{}/f.txt", fwd(&directory))).unwrap();
    let prev = Tree::from([(
        "f.txt".into(),
        Sig {
            size: metadata.size,
            mtime_ms: metadata.mtime_ms,
            hash: 0x5151,
        },
    )]);
    let cached = walk_files(
        &backend,
        &fwd(&directory),
        &cancel,
        &filter,
        HashMode::Full,
        Some(&prev),
    )
    .unwrap();
    assert_eq!(cached["f.txt"].hash, 0x5151);
    let fresh = walk_files(
        &backend,
        &fwd(&directory),
        &cancel,
        &filter,
        HashMode::FullFresh,
        Some(&prev),
    )
    .unwrap();
    assert_eq!(fresh["f.txt"].hash, real);
    std::fs::remove_dir_all(directory).ok();
}

struct FailingHashWalk {
    inner: LocalBackend,
    lists: AtomicUsize,
}

impl Backend for FailingHashWalk {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::Relaxed);
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
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
    fn supports_walk_hashed(&self) -> bool {
        true
    }
    fn walk_hashed(
        &self,
        _root: &str,
        _want_hash: bool,
        tx: crossbeam_channel::Sender<crate::vfs::HashHit>,
        _cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        tx.send(crate::vfs::HashHit {
            rel: "partial.txt".into(),
            is_dir: false,
            size: 7,
            mtime_ms: 1,
            md5: None,
        })
        .unwrap();
        Err(std::io::Error::other("late hash-walk failure"))
    }
}

#[test]
fn partial_agent_hash_walk_error_never_falls_back_to_listing() {
    let directory = tmp("partial-agent-hash");
    let root = fwd(&directory);
    let backend = FailingHashWalk {
        inner: LocalBackend::new(&root),
        lists: AtomicUsize::new(0),
    };
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);

    let error = walk_files(&backend, &root, &cancel, &filter, HashMode::None, None).unwrap_err();
    assert!(error.to_string().contains("late hash-walk failure"));
    assert_eq!(backend.lists.load(Ordering::Relaxed), 0);
    std::fs::remove_dir_all(directory).ok();
}

struct Counting {
    inner: LocalBackend,
    lists: Arc<AtomicUsize>,
}

impl Backend for Counting {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::Relaxed);
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename(source, destination)
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(source, destination)
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }
    fn rename_overwrites(&self) -> bool {
        true
    }
    fn is_local(&self) -> bool {
        true
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
}

#[test]
fn no_op_run_skips_rewalk() {
    let a = tmp("nora");
    let b = tmp("norb");
    std::fs::write(a.join("f.txt"), b"hello").unwrap();
    let (root_a, root_b) = (fwd(&a), fwd(&b));
    let lists_a = Arc::new(AtomicUsize::new(0));
    let lists_b = Arc::new(AtomicUsize::new(0));
    let backend_a = Counting {
        inner: LocalBackend::new(&root_a),
        lists: lists_a.clone(),
    };
    let backend_b = Counting {
        inner: LocalBackend::new(&root_b),
        lists: lists_b.clone(),
    };
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let options = BisyncOptions::default();
    let first = super::super::run(
        &backend_a, &root_a, &backend_b, &root_b, options, &cancel, &filter,
    );
    assert!(first.errors.is_empty());
    assert_eq!(lists_a.load(Ordering::Relaxed), 2);
    assert_eq!(lists_b.load(Ordering::Relaxed), 2);
    lists_a.store(0, Ordering::Relaxed);
    lists_b.store(0, Ordering::Relaxed);
    let second = super::super::run(
        &backend_a, &root_a, &backend_b, &root_b, options, &cancel, &filter,
    );
    assert_eq!(
        second.stats.a_to_b + second.stats.b_to_a + second.stats.deleted,
        0
    );
    assert_eq!(lists_a.load(Ordering::Relaxed), 1);
    assert_eq!(lists_b.load(Ordering::Relaxed), 1);
    let pair = pair_id_for(&backend_a, &root_a, &backend_b, &root_b);
    std::fs::remove_file(baseline_path(&pair)).ok();
    std::fs::remove_dir_all(versions_dir(&pair)).ok();
    std::fs::remove_dir_all(a).ok();
    std::fs::remove_dir_all(b).ok();
}
