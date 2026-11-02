//! Windows publication contracts that need the copy layer: the qualification of the
//! VFS namespace, the held publication callback and the recorded success. Moved here
//! from local_access/os/windows/publication_task_tests.rs: the copy layer lies above
//! the local access layer (plan docs/plaene/2026-11-01-idealzustand, D13). The
//! injected faults stay in local_access, where publication.rs reads them.
use crate::local_access::platform::confined::publication::task_tests::Fault;
use sha2::{Digest, Sha256};
use std::fs;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;

#[test]
fn batches_task_transfer_compat_windows_publication_vfs_keeps_qualification_separate_from_parent_proof(
) {
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
            assert!(error
                .to_string()
                .contains("injected directory flush failure"));
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
fn batches_task_transfer_compat_windows_publication_live_write_through_needs_no_general_parent_proof(
) {
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
            let result =
                crate::copy::transfer_local_observed(&request, &mut |_| {}, &Observe(&cancel));
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
fn batches_task_transfer_compat_windows_publication_recorded_success_stays_bound_to_identity_digest_and_cancel(
) {
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
            &target,
            &identity,
            &digest,
            &AtomicBool::new(true),
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
