use super::*;
use crate::vfs::{Backend, LocalBackend, Scheme, VfsMeta, VfsResult};
use crossbeam_channel::unbounded;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn tmp(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!("sync_{tag}_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn fwd(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn wait(rx: &crossbeam_channel::Receiver<SyncMsg>) -> SyncResult {
    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(SyncMsg::Done(result)) => return result,
            Ok(_) => {}
            Err(_) => panic!("sync timed out"),
        }
    }
}

fn handles(src: &std::path::Path, dst: &std::path::Path) -> (BackendHandle, BackendHandle) {
    (
        Arc::new(LocalBackend::new(&fwd(src))),
        Arc::new(LocalBackend::new(&fwd(dst))),
    )
}

#[test]
fn mirrors_tree_and_updates_changed() {
    let src = tmp("src");
    let dst = tmp("dst");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.txt"), b"hello").unwrap();
    std::fs::write(src.join("sub/b.txt"), b"world!!").unwrap();

    let (sb, db) = handles(&src, &dst);
    let (tx, rx) = unbounded();
    start_sync(
        sb,
        fwd(&src),
        db,
        fwd(&dst),
        SyncOptions {
            delete_extra: false,
            dry_run: false,
        },
        tx,
    );
    let result = wait(&rx);
    assert_eq!(result.stats.errors, 0);
    assert_eq!(result.stats.copied, 2);
    assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"hello");
    assert_eq!(std::fs::read(dst.join("sub/b.txt")).unwrap(), b"world!!");

    std::fs::write(src.join("a.txt"), b"hello world").unwrap();
    let (sb, db) = handles(&src, &dst);
    let (tx, rx) = unbounded();
    start_sync(
        sb,
        fwd(&src),
        db,
        fwd(&dst),
        SyncOptions {
            delete_extra: false,
            dry_run: false,
        },
        tx,
    );
    let result = wait(&rx);
    assert_eq!(result.stats.copied, 1);
    assert_eq!(result.stats.skipped, 1);
    assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"hello world");

    std::fs::remove_dir_all(src).ok();
    std::fs::remove_dir_all(dst).ok();
}

#[test]
fn delete_extra_removes_orphans() {
    let src = tmp("src2");
    let dst = tmp("dst2");
    std::fs::write(src.join("keep.txt"), b"x").unwrap();
    std::fs::write(dst.join("keep.txt"), b"x").unwrap();
    std::fs::write(dst.join("orphan.txt"), b"y").unwrap();
    std::fs::create_dir_all(dst.join("gone")).unwrap();
    std::fs::write(dst.join("gone/z.txt"), b"z").unwrap();

    let (sb, db) = handles(&src, &dst);
    let (tx, rx) = unbounded();
    start_sync(
        sb,
        fwd(&src),
        db,
        fwd(&dst),
        SyncOptions {
            delete_extra: true,
            dry_run: false,
        },
        tx,
    );
    let result = wait(&rx);
    assert!(dst.join("keep.txt").exists());
    assert!(!dst.join("orphan.txt").exists());
    assert!(!dst.join("gone/z.txt").exists());
    assert!(result.stats.deleted >= 2);

    std::fs::remove_dir_all(src).ok();
    std::fs::remove_dir_all(dst).ok();
}

struct FailingSourceStat {
    inner: LocalBackend,
    denied_suffix: String,
}

impl Backend for FailingSourceStat {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if path.ends_with(&self.denied_suffix) {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected source metadata failure",
            ))
        } else {
            self.inner.stat(path)
        }
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
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn is_local(&self) -> bool {
        true
    }
}

#[test]
fn delete_extra_aborts_all_deletes_when_source_probe_fails() {
    let src = tmp("probe_src");
    let dst = tmp("probe_dst");
    std::fs::write(dst.join("protected.txt"), b"keep").unwrap();
    std::fs::write(dst.join("other-orphan.txt"), b"also keep").unwrap();

    let source: BackendHandle = Arc::new(FailingSourceStat {
        inner: LocalBackend::new(&fwd(&src)),
        denied_suffix: "protected.txt".into(),
    });
    let destination: BackendHandle = Arc::new(LocalBackend::new(&fwd(&dst)));
    let (tx, rx) = unbounded();
    start_sync(
        source,
        fwd(&src),
        destination,
        fwd(&dst),
        SyncOptions {
            delete_extra: true,
            dry_run: false,
        },
        tx,
    );
    let result = wait(&rx);
    assert!(result.stats.errors >= 1);
    assert_eq!(result.stats.deleted, 0);
    assert!(dst.join("protected.txt").exists());
    assert!(dst.join("other-orphan.txt").exists());

    std::fs::remove_dir_all(src).ok();
    std::fs::remove_dir_all(dst).ok();
}

#[test]
fn dry_run_writes_nothing() {
    let src = tmp("src3");
    let dst = tmp("dst3");
    std::fs::write(src.join("a.txt"), b"data").unwrap();
    let (sb, db) = handles(&src, &dst);
    let (tx, rx) = unbounded();
    start_sync(
        sb,
        fwd(&src),
        db,
        fwd(&dst),
        SyncOptions {
            delete_extra: false,
            dry_run: true,
        },
        tx,
    );
    let result = wait(&rx);
    assert_eq!(result.stats.copied, 1);
    assert!(!dst.join("a.txt").exists());
    std::fs::remove_dir_all(src).ok();
    std::fs::remove_dir_all(dst).ok();
}

struct HookBackend<'a> {
    inner: &'a LocalBackend,
    cancel_on_list: Option<&'a AtomicBool>,
    appearing_suffix: Option<&'a str>,
    appearance_checks: AtomicUsize,
}

impl Backend for HookBackend<'_> {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let entries = self.inner.list_dir(path)?;
        if let Some(cancel) = self.cancel_on_list {
            cancel.store(true, Ordering::Relaxed);
        }
        Ok(entries)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if self
            .appearing_suffix
            .is_some_and(|suffix| path.ends_with(suffix))
        {
            if self.appearance_checks.fetch_add(1, Ordering::Relaxed) == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "not present during first preflight",
                ));
            }
            return Ok(VfsMeta {
                name: path.rsplit('/').next().unwrap_or(path).into(),
                size: 1,
                ..Default::default()
            });
        }
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
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn is_local(&self) -> bool {
        true
    }
}

#[test]
fn mirror_cancel_during_preflight_deletes_nothing() {
    let source_dir = tmp("cancel_source");
    let destination_dir = tmp("cancel_destination");
    std::fs::write(destination_dir.join("orphan.txt"), b"x").unwrap();
    let source = LocalBackend::new(&fwd(&source_dir));
    let destination = LocalBackend::new(&fwd(&destination_dir));
    let cancel = AtomicBool::new(false);
    let hooked = HookBackend {
        inner: &destination,
        cancel_on_list: Some(&cancel),
        appearing_suffix: None,
        appearance_checks: AtomicUsize::new(0),
    };
    let mut stats = SyncStats::default();
    let mut errors = Vec::new();
    super::super::sync_delete::delete_extras(
        &source,
        &fwd(&source_dir),
        &hooked,
        &fwd(&destination_dir),
        false,
        &cancel,
        &mut stats,
        &mut errors,
    );
    assert_eq!(stats.deleted, 0);
    assert!(!errors.is_empty());
    assert!(destination_dir.join("orphan.txt").exists());
    std::fs::remove_dir_all(source_dir).ok();
    std::fs::remove_dir_all(destination_dir).ok();
}

#[test]
fn source_appearing_after_preflight_blocks_all_deletes() {
    let source_dir = tmp("appearing_source");
    let destination_dir = tmp("appearing_destination");
    std::fs::write(destination_dir.join("appears.txt"), b"x").unwrap();
    std::fs::write(destination_dir.join("other.txt"), b"y").unwrap();
    let source_inner = LocalBackend::new(&fwd(&source_dir));
    let destination = LocalBackend::new(&fwd(&destination_dir));
    let source = HookBackend {
        inner: &source_inner,
        cancel_on_list: None,
        appearing_suffix: Some("appears.txt"),
        appearance_checks: AtomicUsize::new(0),
    };
    let cancel = AtomicBool::new(false);
    let mut stats = SyncStats::default();
    let mut errors = Vec::new();
    super::super::sync_delete::delete_extras(
        &source,
        &fwd(&source_dir),
        &destination,
        &fwd(&destination_dir),
        false,
        &cancel,
        &mut stats,
        &mut errors,
    );
    assert_eq!(stats.deleted, 0);
    assert!(destination_dir.join("appears.txt").exists());
    assert!(destination_dir.join("other.txt").exists());
    std::fs::remove_dir_all(source_dir).ok();
    std::fs::remove_dir_all(destination_dir).ok();
}

#[cfg(unix)]
#[test]
fn link_like_destination_root_never_reaches_external_victim() {
    use std::os::unix::fs::symlink;

    let source = tmp("root_link_source");
    let holder = tmp("root_link_holder");
    let victim = tmp("root_link_victim");
    std::fs::write(victim.join("keep.txt"), b"keep").unwrap();
    let destination_link = holder.join("destination");
    symlink(&victim, &destination_link).unwrap();
    let source_backend: BackendHandle = Arc::new(LocalBackend::new(&fwd(&source)));
    let destination_backend: BackendHandle = Arc::new(LocalBackend::new(&fwd(&destination_link)));
    let (sender, receiver) = unbounded();
    start_sync(
        source_backend,
        fwd(&source),
        destination_backend,
        fwd(&destination_link),
        SyncOptions {
            delete_extra: true,
            dry_run: false,
        },
        sender,
    );
    let result = wait(&receiver);
    assert!(!result.errors.is_empty());
    assert_eq!(result.stats.deleted, 0);
    assert_eq!(std::fs::read(victim.join("keep.txt")).unwrap(), b"keep");
    std::fs::remove_file(destination_link).ok();
    for directory in [source, holder, victim] {
        std::fs::remove_dir_all(directory).ok();
    }
}

#[cfg(unix)]
#[test]
fn link_like_destination_child_never_receives_copied_content() {
    use std::os::unix::fs::symlink;

    let source = tmp("child_link_source");
    let destination = tmp("child_link_destination");
    let victim = tmp("child_link_victim");
    std::fs::create_dir_all(source.join("sub")).unwrap();
    std::fs::write(source.join("sub/file.txt"), b"outside no").unwrap();
    symlink(&victim, destination.join("sub")).unwrap();
    let (source_backend, destination_backend) = handles(&source, &destination);
    let (sender, receiver) = unbounded();
    start_sync(
        source_backend,
        fwd(&source),
        destination_backend,
        fwd(&destination),
        SyncOptions {
            delete_extra: false,
            dry_run: false,
        },
        sender,
    );
    let result = wait(&receiver);
    assert!(!result.errors.is_empty());
    assert!(!victim.join("file.txt").exists());
    std::fs::remove_file(destination.join("sub")).ok();
    for directory in [source, destination, victim] {
        std::fs::remove_dir_all(directory).ok();
    }
}
