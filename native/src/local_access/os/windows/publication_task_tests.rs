//! Windows publication contracts for the single remote task suite.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    fs,
    os::windows::fs::OpenOptionsExt,
    path::Path,
    sync::atomic::AtomicBool,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};

thread_local! {
    static NO_PARENT_PROOF: Cell<bool> = const { Cell::new(false) };
    static FAIL_FLUSH: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(super) fn namespace_support() -> io::Result<()> {
    if NO_PARENT_PROOF.with(Cell::get) {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "injected unavailable parent proof",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn before_flush(file: &File) -> io::Result<()> {
    let Some(expected) = FAIL_FLUSH.with(|value| value.borrow().clone()) else {
        return Ok(());
    };
    if expected == super::super::identity(file)? {
        return Err(io::Error::other("injected directory flush failure"));
    }
    Ok(())
}

struct Fault;
impl Fault {
    fn missing_parent_proof() -> Self {
        NO_PARENT_PROOF.with(|flag| assert!(!flag.replace(true)));
        Self
    }

    fn flush(file: &File) -> Self {
        FAIL_FLUSH.with(|value| {
            assert!(value
                .borrow_mut()
                .replace(super::super::identity(file).unwrap())
                .is_none());
        });
        Self
    }
}
impl Drop for Fault {
    fn drop(&mut self) {
        NO_PARENT_PROOF.with(|flag| flag.set(false));
        FAIL_FLUSH.with(|value| *value.borrow_mut() = None);
    }
}

fn raw_directory(path: &Path, sharing: u32) -> File {
    fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | 0x0010_0000)
        .share_mode(sharing)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap()
}

#[test]
fn batches_task_transfer_compat_windows_publication_flushes_root_and_native_child() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("child")).unwrap();
    let parent = crate::local_access::ConfinedDirectory::open_root(root.path()).unwrap();
    let child = parent.child(OsStr::new("child")).unwrap();
    for directory in [&parent, &child] {
        let before = directory.identity().unwrap();
        directory.confirm_publication_namespace().unwrap();
        assert_eq!(directory.identity().unwrap(), before);
    }
}

#[test]
fn batches_task_transfer_compat_windows_publication_reopens_the_held_directory_after_rename() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("chosen");
    let moved = root.path().join("moved");
    fs::create_dir(&original).unwrap();
    let held = raw_directory(
        &original,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    );
    let identity = super::super::identity(&held).unwrap();
    // Only this explicit test handle allows its directory name to change.
    // Production ConfinedDirectory pins continue to deny delete sharing.
    fs::rename(&original, &moved).unwrap();
    fs::create_dir(&original).unwrap();
    let foreign = raw_directory(
        &original,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    );
    let reopened = reopen_directory(&held).unwrap();
    assert_eq!(super::super::identity(&reopened).unwrap(), identity);
    assert_ne!(super::super::identity(&foreign).unwrap(), identity);
    confirm_namespace(&held).unwrap();
}

#[test]
fn batches_task_transfer_compat_windows_publication_rejects_wrong_type_and_write_reservation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("regular.bin");
    fs::write(&path, b"unchanged").unwrap();
    let file = File::open(&path).unwrap();
    assert_eq!(
        reopen_directory(&file).unwrap_err().kind(),
        io::ErrorKind::InvalidData,
    );
    let held = raw_directory(root.path(), FILE_SHARE_READ);
    let error = reopen_directory(&held).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32), "{error}");
    assert_eq!(fs::read(&path).unwrap(), b"unchanged");
}

#[test]
fn batches_task_transfer_compat_windows_publication_never_promotes_non_ntfs_or_remote_labels() {
    assert!(namespace_supported("NTFS", 0));
    assert!(namespace_supported("ntfs", 0));
    for filesystem in [
        "ReFS", "FAT", "FAT12", "FAT16", "FAT32", "exFAT", "CDFS", "UDF", "",
    ] {
        assert!(!namespace_supported(filesystem, 0), "{filesystem}");
    }
    assert!(!namespace_supported("NTFS", 0x10));
    assert!(!namespace_supported("NTFS", 0x2000));
}

#[test]
fn batches_task_transfer_compat_windows_publication_propagates_directory_flush_failures() {
    let root = tempfile::tempdir().unwrap();
    let parent = crate::local_access::ConfinedDirectory::open_root(root.path()).unwrap();
    let identity = parent.identity().unwrap();
    let read_only = raw_directory(root.path(), FILE_SHARE_READ | FILE_SHARE_WRITE);
    let error = flush_directory(read_only).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(5), "{error}");
    {
        let _fault = Fault::flush(parent.file());
        let error = parent.confirm_publication_namespace().unwrap_err();
        assert!(error.to_string().contains("injected directory flush failure"));
        assert_eq!(parent.identity().unwrap(), identity);
    }
    parent.confirm_publication_namespace().unwrap();
}

#[test]
fn batches_task_transfer_compat_windows_publication_vfs_keeps_qualification_separate_from_parent_proof() {
    let root = tempfile::tempdir().unwrap();
    let path = crate::copy::native_path_text(root.path()).unwrap();
    let local = crate::vfs::LocalBackend::new("/");
    let confined = crate::vfs::BackendExtensions::confined_local_root(&local, &path)
        .unwrap()
        .unwrap();
    let parent = crate::local_access::ConfinedDirectory::open_root(root.path()).unwrap();
    for backend in [&local as &dyn crate::vfs::Backend, confined.as_ref()] {
        assert!(crate::vfs::confirm_namespace(backend, &path).unwrap());
        {
            let _fault = Fault::missing_parent_proof();
            assert!(crate::vfs::confirm_namespace(backend, &path).unwrap());
            assert_eq!(
                parent.confirm_publication_namespace().unwrap_err().kind(),
                io::ErrorKind::Unsupported,
            );
        }
        {
            let _fault = Fault::flush(parent.file());
            // VFS qualifies the established publication contract. Only the
            // independent parent-proof API actually attempts this flush.
            assert!(crate::vfs::confirm_namespace(backend, &path).unwrap());
            let error = parent.confirm_publication_namespace().unwrap_err();
            assert!(error.to_string().contains("injected directory flush failure"));
        }
        assert!(crate::vfs::confirm_namespace(backend, &path).unwrap());
    }
}

struct Observe<'a>(&'a AtomicBool);
impl crate::copy::PublicationObserver for Observe<'_> {
    fn before_publish(&self, _: &Path, _: &Path) -> io::Result<()> {
        Ok(())
    }

    fn after_publish(&self, _: &Path) -> io::Result<()> {
        Err(io::Error::other("held publication callback was bypassed"))
    }

    fn after_publish_handle(&self, target: &Path, file: &File) -> io::Result<()> {
        crate::copy::confirm_transfer_publication_handle(
            target,
            file,
            &crate::copy::local_file_identity_handle(file)?,
            &format!("{:x}", Sha256::digest(b"payload")),
            self.0,
        )
    }
}

#[test]
fn batches_task_transfer_compat_windows_publication_live_write_through_needs_no_general_parent_proof() {
    // This injects missing directory-proof capability on the runner's actual
    // filesystem. It is not a claim that this fixture is a real FAT volume.
    let _fault = Fault::missing_parent_proof();
    for moving in [false, true] {
        for readonly in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.bin");
            let target = root.path().join("target.bin");
            fs::write(&source, b"payload").unwrap();
            let mut permissions = fs::metadata(&source).unwrap().permissions();
            permissions.set_readonly(readonly);
            fs::set_permissions(&source, permissions).unwrap();
            let cancel = AtomicBool::new(false);
            let request = crate::copy::LocalRequest {
                source: &source,
                target: &target,
                conflict: crate::types::Conflict::Overwrite,
                mode: if moving {
                    crate::types::CopyMode::Move
                } else {
                    crate::types::CopyMode::Copy
                },
                cancel: &cancel,
                parallel: 1,
                probe: None,
            };
            let result = crate::copy::transfer_local_observed(
                &request,
                &mut |_| {},
                &Observe(&cancel),
            );
            assert!(
                matches!(result, Ok(crate::copy::LocalOutcome::Completed { .. })),
                "{result:?}",
            );
            assert_eq!(fs::read(&target).unwrap(), b"payload");
            assert_eq!(source.exists(), !moving);
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().readonly(),
                readonly,
            );
            for path in [&source, &target] {
                if path.exists() {
                    let mut permissions = fs::metadata(path).unwrap().permissions();
                    permissions.set_readonly(false);
                    fs::set_permissions(path, permissions).unwrap();
                }
            }
        }
    }
}

#[test]
fn batches_task_transfer_compat_windows_publication_recorded_success_stays_bound_to_identity_digest_and_cancel() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    fs::write(&source, b"payload").unwrap();
    let cancel = AtomicBool::new(false);
    let request = crate::copy::LocalRequest {
        source: &source,
        target: &target,
        conflict: crate::types::Conflict::Rename,
        mode: crate::types::CopyMode::Move,
        cancel: &cancel,
        parallel: 1,
        probe: None,
    };
    crate::copy::transfer_local(&request, &mut |_| {}).unwrap();
    let identity = crate::copy::local_file_identity(&target).unwrap();
    let digest = format!("{:x}", Sha256::digest(b"payload"));
    let _fault = Fault::missing_parent_proof();
    assert_eq!(
        crate::copy::confirm_transfer_publication(&target, &identity, &digest, &cancel)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported,
    );
    crate::copy::confirm_acknowledged_transfer_publication(&target, &identity, &digest, &cancel)
        .unwrap();
    let wrong = format!("{:x}", Sha256::digest(b"foreign"));
    assert!(crate::copy::confirm_acknowledged_transfer_publication(
        &target, &identity, &wrong, &cancel,
    )
    .is_err());
    assert_eq!(
        crate::copy::confirm_acknowledged_transfer_publication(
            &target, &identity, &digest, &AtomicBool::new(true),
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted,
    );
    fs::rename(&target, root.path().join("retained-own-object")).unwrap();
    fs::write(&target, b"payload").unwrap();
    assert!(crate::copy::confirm_acknowledged_transfer_publication(
        &target, &identity, &digest, &cancel,
    )
    .is_err());
    assert_eq!(fs::read(&target).unwrap(), b"payload");
    assert_eq!(
        fs::read(root.path().join("retained-own-object")).unwrap(),
        b"payload",
    );
}
