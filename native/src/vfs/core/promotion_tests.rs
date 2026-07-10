use super::{promote_staged_create, promote_staged_replace, Backend, LocalBackend};

fn root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("se_promote_{name}_{}", std::process::id()))
}

#[test]
fn promotion_replaces_only_after_staged_file_exists() {
    let root = root("replace");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let destination = root.join("file");
    let staged = root.join("file.stage");
    std::fs::write(&destination, b"old").unwrap();
    std::fs::write(&staged, b"new").unwrap();
    let backend = LocalBackend::new("/");
    promote_staged_replace(
        &backend,
        staged.to_str().unwrap(),
        destination.to_str().unwrap(),
    )
    .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    assert!(!staged.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn failed_promotion_restores_old_destination() {
    let root = root("rollback");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let destination = root.join("file");
    let missing_staged = root.join("missing.stage");
    std::fs::write(&destination, b"old").unwrap();
    let backend = LocalBackend::new("/");
    assert!(promote_staged_replace(
        &backend,
        missing_staged.to_str().unwrap(),
        destination.to_str().unwrap(),
    )
    .is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn local_no_replace_preserves_existing_destination() {
    let root = root("noreplace");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("source");
    let destination = root.join("destination");
    std::fs::write(&source, b"source").unwrap();
    std::fs::write(&destination, b"destination").unwrap();
    let backend = LocalBackend::new("/");
    assert!(backend
        .rename_no_replace(source.to_str().unwrap(), destination.to_str().unwrap())
        .is_err());
    assert_eq!(std::fs::read(&source).unwrap(), b"source");
    assert_eq!(std::fs::read(&destination).unwrap(), b"destination");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn create_promotion_preserves_a_destination_that_appeared_after_preflight() {
    let root = root("create_racer");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let staged = root.join("stage");
    let destination = root.join("destination");
    std::fs::write(&staged, b"new").unwrap();
    // This represents a second writer creating the name after the caller's
    // absence check but before commit.
    std::fs::write(&destination, b"racer").unwrap();

    let backend = LocalBackend::new("/");
    let error = promote_staged_create(
        &backend,
        staged.to_str().unwrap(),
        destination.to_str().unwrap(),
    )
    .unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&destination).unwrap(), b"racer");
    assert_eq!(std::fs::read(&staged).unwrap(), b"new");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn promotion_never_replaces_an_existing_directory() {
    let root = root("directory_type");
    let _ = std::fs::remove_dir_all(&root);
    let destination = root.join("directory");
    std::fs::create_dir_all(destination.join("child")).unwrap();
    std::fs::write(destination.join("child/file"), b"keep").unwrap();
    let staged = root.join("stage");
    std::fs::write(&staged, b"new file").unwrap();

    let backend = LocalBackend::new("/");
    let error = promote_staged_replace(
        &backend,
        staged.to_str().unwrap(),
        destination.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        std::fs::read(destination.join("child/file")).unwrap(),
        b"keep"
    );
    assert_eq!(std::fs::read(&staged).unwrap(), b"new file");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn default_no_replace_never_calls_a_replacing_rename() {
    use super::{Scheme, VfsMeta, VfsResult};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct UnsafeRenameBackend(AtomicUsize);
    impl Backend for UnsafeRenameBackend {
        fn scheme(&self) -> Scheme {
            Scheme::Ftp
        }
        fn root_display(&self) -> String {
            "/".into()
        }
        fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
            Ok(Vec::new())
        }
        fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
            Err(std::io::ErrorKind::NotFound.into())
        }
        fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
            Err(std::io::ErrorKind::Unsupported.into())
        }
        fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
            Err(std::io::ErrorKind::Unsupported.into())
        }
        fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn remove_file(&self, _path: &str) -> VfsResult<()> {
            Ok(())
        }
        fn remove_dir(&self, _path: &str) -> VfsResult<()> {
            Ok(())
        }
        fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
            Ok(())
        }
    }

    let backend = UnsafeRenameBackend(AtomicUsize::new(0));
    let error = backend
        .rename_no_replace("stage", "destination")
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert_eq!(backend.0.load(Ordering::SeqCst), 0);
}
