//! Existing provider paths remain usable without restart-stable identities.
use super::{
    test_backend::{fwd, job, run, write},
    *,
};
use crate::vfs::{Backend, BackendExtensions, LocalBackend, Scheme, VfsMeta};
use std::{
    fs,
    io::{self, Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

pub(super) struct Limited {
    pub inner: LocalBackend,
    pub identity: String,
    pub read_error: Option<io::ErrorKind>,
    pub lost_directory: AtomicBool,
    pub lost_publication: AtomicBool,
    pub creates: AtomicUsize,
    pub promotes: AtomicUsize,
    pub proofs: bool,
    pub refuse_reserved: bool,
    pub reserved_renames: AtomicUsize,
    pub mutate_stage: AtomicUsize,
    pub refuse_listing: AtomicBool,
    /// The device behind this target hashes files itself (an SSH server that
    /// runs `sha256sum`, an FTP server with `HASH`, Drive's checksum).
    pub device_digest: bool,
    pub digests: AtomicUsize,
    /// The time resolution the target reports (`target_limits`).
    pub precision: crate::vfs::MtimePrecision,
    /// `rename` replaces an existing file atomically (`rename_overwrites`):
    /// only then may the file rule replace an older target; without it the
    /// replacement is refused and both are kept (`vfs/core/promotion.rs`).
    pub replaces: bool,
}
impl Limited {
    pub fn new(label: &str) -> Self {
        Self {
            inner: LocalBackend::new("/"),
            identity: super::test_backend::unique(label),
            read_error: None,
            lost_directory: AtomicBool::new(false),
            lost_publication: AtomicBool::new(false),
            creates: AtomicUsize::new(0),
            promotes: AtomicUsize::new(0),
            proofs: false,
            refuse_reserved: false,
            reserved_renames: AtomicUsize::new(0),
            mutate_stage: AtomicUsize::new(0),
            refuse_listing: AtomicBool::new(false),
            device_digest: false,
            digests: AtomicUsize::new(0),
            precision: crate::vfs::MtimePrecision::Unknown,
            replaces: false,
        }
    }
}
impl Backend for Limited {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        self.identity.clone()
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        if self.refuse_listing.swap(false, Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "listing initially refused",
            ));
        }
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> io::Result<VfsMeta> {
        if path.contains(".se-upload-") && std::path::Path::new(path).is_file() {
            match self.mutate_stage.swap(0, Ordering::SeqCst) {
                1 => fs::remove_file(path)?,
                2 => fs::remove_file(path).and_then(|()| fs::create_dir(path))?,
                3 => fs::write(path, b"changed size after STOR")?,
                _ => {}
            }
        }
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
        if let Some(kind) = self.read_error {
            return Err(io::Error::new(kind, "fixture target read refused"));
        }
        self.inner.open_read(path)
    }
    fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> {
        Err(io::Error::other("replacing writer forbidden"))
    }
    fn open_write_new(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.inner.open_write_new(path)
    }
    fn promote_copy_stage(&self, source: &str, destination: &str) -> io::Result<()> {
        self.promotes.fetch_add(1, Ordering::SeqCst);
        self.inner.promote_copy_stage(source, destination)?;
        if self.lost_publication.swap(false, Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "publication response lost",
            ));
        }
        Ok(())
    }
    fn rename(&self, source: &str, destination: &str) -> io::Result<()> {
        self.inner.rename(source, destination)
    }
    fn rename_overwrites(&self) -> bool {
        self.replaces
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> io::Result<()> {
        self.inner.rename_no_replace(source, destination)
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
    fn create_dir_new(&self, path: &str) -> io::Result<()> {
        self.inner.create_dir_new(path)
    }
    fn discard_copy_stage(&self, path: &str) -> io::Result<()> {
        self.inner.discard_copy_stage(path)
    }
}
impl BackendExtensions for Limited {
    fn directory_reservation_supported(&self, _: &str) -> bool {
        true
    }
    fn create_dir_exclusive(&self, path: &str) -> io::Result<()> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        crate::vfs::create_dir_exclusive(&self.inner, path)?;
        if self.lost_directory.swap(false, Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "mkdir response lost",
            ));
        }
        Ok(())
    }
    fn directory_identity(&self, path: &str) -> io::Result<Option<String>> {
        if self.proofs {
            crate::vfs::directory_identity(&self.inner, path)
        } else {
            Ok(None)
        }
    }
    fn file_identity(&self, path: &str) -> io::Result<Option<String>> {
        if self.proofs {
            crate::vfs::file_identity(&self.inner, path)
        } else {
            Ok(None)
        }
    }
    fn target_limits(&self, _root: &str) -> crate::vfs::TargetLimits {
        crate::vfs::TargetLimits {
            mtime_precision: self.precision,
            ..crate::vfs::TargetLimits::default()
        }
    }
    fn content_digest(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> io::Result<Option<crate::vfs::ContentDigest>> {
        if !self.device_digest {
            return Ok(None);
        }
        self.digests.fetch_add(1, Ordering::SeqCst);
        // The device reads its own disk: `open_read` of this stand-in, which
        // may be refused for the client, is not involved.
        crate::vfs::digest_by_local_read(&self.inner, path, cancel)
    }
    fn rename_reserved_directory(&self, source: &str, destination: &str) -> io::Result<()> {
        self.reserved_renames.fetch_add(1, Ordering::SeqCst);
        if self.refuse_reserved {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no atomic binding rename",
            ));
        }
        self.inner.rename_no_replace(source, destination)
    }
}

#[test]
fn batches_task_transfer_compat_folder_identity_does_not_require_binding_rename_for_first_copy() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Photos");
    let target = root.path().join("target");
    write(&source.join("a.bin"), b"ours");
    fs::create_dir(&target).unwrap();
    let mut limited = Limited::new("id-without-binding");
    limited.proofs = true;
    limited.refuse_reserved = true;
    assert!(
        crate::vfs::directory_identity(&limited.inner, &fwd(&target))
            .unwrap()
            .is_some()
    );
    let backend = Arc::new(limited);
    let result = run(job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend.clone()),
        &target,
        &[&source],
    ));
    assert!(result.issues.is_empty(), "{:?}", result.issues);
    assert_eq!(backend.reserved_renames.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(fs::read(target.join("Photos/a.bin")).unwrap(), b"ours");
}

#[test]
fn batches_task_transfer_compat_folder_without_identity_copies_and_numbers() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("from/Photos");
    let target = root.path().join("target");
    write(&source.join("nested/a.bin"), b"ours");
    write(&target.join("Photos/foreign"), b"foreign");
    let backend = Arc::new(Limited::new("folder-no-id"));
    let outcome = run(job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend.clone()),
        &target,
        &[&source],
    ));
    assert!(outcome.issues.is_empty(), "{:?}", outcome.issues);
    assert_eq!(outcome.roots[0].rel, "Photos (2)");
    assert_eq!(
        fs::read(target.join("Photos (2)/nested/a.bin")).unwrap(),
        b"ours"
    );
    assert_eq!(fs::read(target.join("Photos/foreign")).unwrap(), b"foreign");
    assert_eq!(backend.creates.load(Ordering::SeqCst), 1);
}

#[test]
fn batches_task_transfer_compat_folder_lost_response_is_not_recovered_without_identity() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Photos");
    let target = root.path().join("target");
    write(&source.join("a.bin"), b"ours");
    fs::create_dir(&target).unwrap();
    let backend = Arc::new(Limited::new("folder-lost-no-id"));
    backend.lost_directory.store(true, Ordering::SeqCst);
    let mut candidate = job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend.clone()),
        &target,
        &[&source],
    );
    let first = run(candidate.clone());
    assert!(!first.issues.is_empty());
    candidate.recovery = crate::transfer::RecoveryMode::Resume;
    let second = run(candidate);
    assert!(!second.issues.is_empty());
    assert_eq!(backend.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(fs::read_dir(target.join("Photos")).unwrap().count(), 0);
}

#[test]
fn batches_task_transfer_compat_missing_only_preserves_resolved_folder_and_skip_contract() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("from/Photos");
    let target = root.path().join("target");
    write(&source.join("a.bin"), b"aaa");
    write(&source.join("b.bin"), b"bbb");
    write(&source.join("c.bin"), b"ccc");
    write(&target.join("Photos/foreign"), b"foreign");
    let backend = Arc::new(Limited::new("missing-no-id"));
    let mut candidate = job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend.clone()),
        &target,
        &[&source],
    );
    let first = run(candidate.clone());
    assert!(first.issues.is_empty(), "{:?}", first.issues);
    fs::remove_file(target.join("Photos (2)/b.bin")).unwrap();
    write(&target.join("Photos (2)/c.bin"), b"different length");
    candidate.recovery = crate::transfer::RecoveryMode::Resume;
    candidate.resume = Some(first.roots);
    let previous = backend.promotes.load(Ordering::SeqCst);
    let second = run(candidate);
    assert_eq!(backend.promotes.load(Ordering::SeqCst) - previous, 1);
    assert_eq!(fs::read(target.join("Photos (2)/a.bin")).unwrap(), b"aaa");
    assert_eq!(fs::read(target.join("Photos (2)/b.bin")).unwrap(), b"bbb");
    assert_eq!(
        fs::read(target.join("Photos (2)/c.bin")).unwrap(),
        b"different length"
    );
    assert!(!target.join("Photos (3)").exists());
    assert_eq!(second.progress.errors, 1);
}

#[test]
fn batches_task_transfer_compat_missing_only_preserves_skipped_selected_folder() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    let folder = source.join("Photos");
    let single = source.join("a.bin");
    write(&folder.join("ours.bin"), b"ours");
    write(&single, b"aaa");
    write(&target.join("Photos/foreign"), b"foreign");
    let backend = Arc::new(Limited::new("missing-skip-folder"));
    let mut candidate = job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend),
        &target,
        &[&folder, &single],
    );
    candidate
        .folders
        .roots
        .insert(fwd(&folder), crate::transfer::FolderConflict::Skip);
    let first = run(candidate.clone());
    assert!(first.issues.is_empty(), "{:?}", first.issues);
    fs::remove_file(target.join("a.bin")).unwrap();
    candidate.recovery = crate::transfer::RecoveryMode::Resume;
    candidate.resume = Some(first.roots);
    let retry = run(candidate);
    assert!(retry.issues.is_empty(), "{:?}", retry.issues);
    assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"aaa");
    assert!(!target.join("Photos/ours.bin").exists());
    assert_eq!(fs::read(target.join("Photos/foreign")).unwrap(), b"foreign");
}

#[test]
fn batches_task_transfer_compat_missing_only_remote_pairs_keep_literal_names_and_endpoints() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    write(&source.join("a#%.bin"), b"aaa");
    write(&source.join("b.bin"), b"bbb");
    fs::create_dir(&target).unwrap();
    let one = Arc::new(Limited::new("remote-source"));
    let two = Arc::new(Limited::new("remote-target"));
    let mut candidate = job(
        crate::transfer::Endpoint::Remote(one),
        crate::transfer::Endpoint::Remote(two.clone()),
        &target,
        &[],
    );
    candidate.items = crate::transfer::JobItems::Pairs(vec![
        crate::transfer::PairItem::new(fwd(&source.join("a#%.bin")), "literal/a#%.bin"),
        crate::transfer::PairItem::new(fwd(&source.join("b.bin")), "literal/b.bin"),
    ]);
    let first = run(candidate.clone());
    assert!(first.issues.is_empty(), "{:?}", first.issues);
    fs::remove_file(target.join("literal/b.bin")).unwrap();
    candidate.recovery = crate::transfer::RecoveryMode::Resume;
    candidate.resume = Some(first.roots);
    let previous = two.promotes.load(Ordering::SeqCst);
    let second = run(candidate);
    assert!(second.issues.is_empty(), "{:?}", second.issues);
    assert_eq!(two.promotes.load(Ordering::SeqCst) - previous, 1);
    assert_eq!(fs::read(target.join("literal/a#%.bin")).unwrap(), b"aaa");
    assert_eq!(fs::read(target.join("literal/b.bin")).unwrap(), b"bbb");
}
