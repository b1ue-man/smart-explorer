//! Removable media (2026-11-27, U1/U3): the no-replace fallback of capture
//! and restore, and no capture folder left behind on a volume whose owner
//! and mode come from the mount options.
use super::*;
use std::io::Read;

fn flag_refusal() -> io::Error {
    io::Error::from_raw_os_error(libc::EINVAL)
}

/// The link step of the ladder moves the entry without replacing a name:
/// a taken destination is refused and both entries keep their bytes.
#[test]
fn removable_task_capture_link_fallback_never_replaces() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir(fixture.path().join("from")).unwrap();
    std::fs::create_dir(fixture.path().join("to")).unwrap();
    std::fs::write(fixture.path().join("from/a"), b"first").unwrap();
    std::fs::write(fixture.path().join("from/c"), b"third").unwrap();
    let from = DirectoryHandle::open_root(&fixture.path().join("from")).unwrap();
    let to = DirectoryHandle::open_root(&fixture.path().join("to")).unwrap();
    // The source directory moves meanwhile: the held descriptors still name it.
    std::fs::rename(fixture.path().join("from"), fixture.path().join("moved")).unwrap();

    link_then_unlink(
        &from,
        &CString::new("a").unwrap(),
        &to,
        &CString::new("b").unwrap(),
        &flag_refusal(),
    )
    .unwrap();
    assert!(!fixture.path().join("moved/a").exists());
    assert_eq!(
        std::fs::read(fixture.path().join("to/b")).unwrap(),
        b"first"
    );

    let error = link_then_unlink(
        &from,
        &CString::new("c").unwrap(),
        &to,
        &CString::new("b").unwrap(),
        &flag_refusal(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read(fixture.path().join("moved/c")).unwrap(),
        b"third"
    );
    assert_eq!(
        std::fs::read(fixture.path().join("to/b")).unwrap(),
        b"first"
    );
}

/// The directories of the suite's FAT and exFAT images (mounted through a
/// loop device as udisks2 does: `uid=`/`gid=` of the test user,
/// `dmask=0022,fmask=0133`, so a new folder shows 0755).
fn mount_mode_volumes() -> Vec<std::path::PathBuf> {
    ["SE_TEST_VFAT_DIR", "SE_TEST_EXFAT_DIR"]
        .iter()
        .map(|name| {
            std::path::PathBuf::from(
                std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")),
            )
        })
        .collect()
}

/// A fresh folder of its own on `volume`, which must show the mount's mode
/// instead of a private one (otherwise the test proves nothing).
fn fresh_folder(volume: &std::path::Path, label: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let folder = volume.join(format!("se-core-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir(&folder).unwrap();
    let probe = folder.join("probe");
    std::fs::create_dir(&probe).unwrap();
    let mode = std::fs::metadata(&probe).unwrap().permissions().mode();
    std::fs::remove_dir(&probe).unwrap();
    assert_ne!(
        mode & 0o077,
        0,
        "{}: mount the volume with dmask=0022 (mode {mode:o})",
        volume.display()
    );
    folder
}

/// U1: where the capture folder cannot be made private, the refusal leaves
/// no `.held.se-recycle-<hex>` folder and the file where it was.
#[test]
#[ignore = "needs a mounted vfat (SE_TEST_VFAT_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory; Suite crates-task"]
fn removable_task_refused_capture_leaves_no_capture_folder() {
    for volume in mount_mode_volumes() {
        let folder = fresh_folder(&volume, "capture");
        std::fs::write(folder.join("file"), b"kept").unwrap();
        let handle = DirectoryHandle::open_root(&folder).unwrap();
        let expected = std::fs::File::open(folder.join("file")).unwrap();
        let error = handle
            .quarantine_regular_child(OsStr::new("file"), &expected)
            .err()
            .expect("a capture folder with the mount's mode is not owner-private");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        let names: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            names,
            vec![std::ffi::OsString::from("file")],
            "{}",
            volume.display()
        );
        let mut content = String::new();
        std::fs::File::open(folder.join("file"))
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "kept");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
