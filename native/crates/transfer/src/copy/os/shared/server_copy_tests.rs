//! Kernel copies into a new stage: the server-side copy of connections that
//! are local filesystems underneath (UNC shares), exercised through
//! `LocalBackend::server_copy_to_stage` as the transfer engine calls it.
use crate::vfs::{Backend, LocalBackend};
use std::fs;
use std::io;
use std::path::Path;

fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn content() -> Vec<u8> {
    (0..300_000u32).map(|index| (index % 241) as u8).collect()
}

#[test]
fn transfer_engine_task_local_server_copy_fills_a_new_stage() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("source.bin");
    let bytes = content();
    fs::write(&source, &bytes).expect("source");
    let stage = root.path().join("copy.bin.se-upload-0123456789abcdef");
    let copied = LocalBackend::new("/")
        .server_copy_to_stage(
            &fwd(&source),
            &fwd(&stage),
            bytes.len() as u64,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect("server copy");
    assert_eq!(copied, Some(bytes.len() as u64));
    assert_eq!(fs::read(&stage).expect("stage"), bytes);
    let metadata = fs::symlink_metadata(&stage).expect("stage metadata");
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // CopyFile2 wrote a plain file, not a reparse point.
        assert_eq!(metadata.file_attributes() & 0x400, 0);
    }
    assert_eq!(fs::read(&source).expect("source stays"), bytes);
}

#[test]
fn transfer_engine_task_local_server_copy_never_adopts_an_existing_stage() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("source.bin");
    fs::write(&source, b"new bytes").expect("source");
    let stage = root.path().join("taken.se-upload-0123456789abcdef");
    fs::write(&stage, b"foreign").expect("foreign stage");
    let error = LocalBackend::new("/")
        .server_copy_to_stage(
            &fwd(&source),
            &fwd(&stage),
            9,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect_err("an existing stage name is never used");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&stage).expect("foreign data"), b"foreign");
}

#[test]
fn transfer_engine_task_local_server_copy_checks_the_expected_length() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("source.bin");
    let bytes = content();
    fs::write(&source, &bytes).expect("source");
    let stage = root.path().join("short.se-upload-0123456789abcdef");
    let error = LocalBackend::new("/")
        .server_copy_to_stage(
            &fwd(&source),
            &fwd(&stage),
            bytes.len() as u64 + 1,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect_err("a copy of another length than listed is refused");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("geändert"), "{error}");
    assert!(!stage.exists(), "the refused copy is removed");
    assert_eq!(fs::read(&source).expect("source stays"), bytes);
}

#[test]
fn transfer_engine_task_local_server_copy_refuses_folders() {
    let root = tempfile::tempdir().expect("temp dir");
    let folder = root.path().join("folder");
    fs::create_dir(&folder).expect("folder");
    let stage = root.path().join("folder.se-upload-0123456789abcdef");
    let error = LocalBackend::new("/")
        .server_copy_to_stage(
            &fwd(&folder),
            &fwd(&stage),
            0,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect_err("only regular files are copied");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(!stage.exists());
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_local_server_copy_refuses_links() {
    let root = tempfile::tempdir().expect("temp dir");
    let target = root.path().join("target.bin");
    fs::write(&target, b"behind the link").expect("link target");
    let link = root.path().join("link.bin");
    std::os::unix::fs::symlink(&target, &link).expect("link");
    let stage = root.path().join("link.se-upload-0123456789abcdef");
    let error = LocalBackend::new("/")
        .server_copy_to_stage(
            &fwd(&link),
            &fwd(&stage),
            15,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect_err("a link as source is refused, not followed");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(
        error.to_string().contains("keine reguläre Datei"),
        "{error}"
    );
    assert!(!stage.exists());
}
