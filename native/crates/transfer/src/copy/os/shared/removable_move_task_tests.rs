//! Moves from and onto removable Windows volumes (U13, U17, 2026-11-27).
//! The suite formats a VHD as FAT32 and one as exFAT and names a writable
//! folder on each in `SE_TEST_FAT32_DIR` and `SE_TEST_EXFAT_DIR`; the other
//! volume of a cross-volume move is the runner's NTFS temp folder.
use crate::copy::{move_folder, transfer_local, LocalOutcome, LocalRequest};
use crate::types::{Conflict, CopyMode};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

const NEED: &str = "needs a FAT32 (SE_TEST_FAT32_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory on Windows; Suite crates-task";

fn fresh(base: PathBuf, label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = base.join(format!(
        "se-removable-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn volume_dir(variable: &str, label: &str) -> PathBuf {
    let base =
        std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} is not set ({NEED})"));
    fresh(PathBuf::from(base), label)
}

fn removable_dirs(label: &str) -> [PathBuf; 2] {
    [
        volume_dir("SE_TEST_FAT32_DIR", label),
        volume_dir("SE_TEST_EXFAT_DIR", label),
    ]
}

fn move_one(source: &Path, target: &Path) {
    let cancel = AtomicBool::new(false);
    let request = LocalRequest {
        source,
        target,
        conflict: Conflict::Skip,
        mode: CopyMode::Move,
        cancel: &cancel,
        parallel: 1,
        probe: None,
    };
    match transfer_local(&request, &mut |_| {}) {
        Ok(LocalOutcome::Completed {
            target: reached, ..
        }) => assert_eq!(reached, target),
        other => panic!(
            "move {} -> {}: {other:?}",
            source.display(),
            target.display()
        ),
    }
    assert!(
        std::fs::symlink_metadata(source).is_err(),
        "the source {} is gone",
        source.display()
    );
}

/// No quarantine or stage name is left in `dir` after a finished move.
fn assert_clean(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            !name.contains(".smart-explorer-") && !name.contains(".se-move-publish-"),
            "leftover {name} in {}",
            dir.display()
        );
    }
}

#[test]
#[ignore = "needs a FAT32 (SE_TEST_FAT32_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory on Windows; Suite crates-task"]
fn removable_task_move_file_within_and_across_fat_volumes() {
    let other = fresh(std::env::temp_dir(), "ntfs");
    let [fat32, exfat] = removable_dirs("file");
    for dir in [&fat32, &exfat] {
        // A short 8.3 name takes one directory entry; the quarantine name
        // needs long-name entries, which moves the entry (and its number).
        let source = dir.join("a.txt");
        std::fs::write(&source, b"within").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let within = dir.join("sub").join("a.txt");
        move_one(&source, &within);
        assert_eq!(std::fs::read(&within).unwrap(), b"within");

        let away = other.join(format!(
            "{}.txt",
            dir.file_name().unwrap().to_string_lossy()
        ));
        move_one(&within, &away);
        assert_eq!(std::fs::read(&away).unwrap(), b"within");

        let back = dir.join("b.txt");
        move_one(&away, &back);
        assert_eq!(std::fs::read(&back).unwrap(), b"within");
        assert_clean(dir);
        assert_clean(&dir.join("sub"));
    }
    // From one removable volume to the other.
    let source = fat32.join("c.txt");
    std::fs::write(&source, b"fat32 to exfat").unwrap();
    let target = exfat.join("c.txt");
    move_one(&source, &target);
    assert_eq!(std::fs::read(&target).unwrap(), b"fat32 to exfat");
    assert_clean(&fat32);
    assert_clean(&other);
    for dir in [fat32, exfat, other] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
#[ignore = "needs a FAT32 (SE_TEST_FAT32_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory on Windows; Suite crates-task"]
fn removable_task_move_folder_within_a_fat_volume() {
    for dir in removable_dirs("folder") {
        let folder = dir.join("photos");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join("inner").join("p.jpg"), b"picture").unwrap();
        let target = dir.join("moved photos");
        move_folder(&folder, &target).unwrap();
        assert!(std::fs::symlink_metadata(&folder).is_err());
        assert_eq!(
            std::fs::read(target.join("inner").join("p.jpg")).unwrap(),
            b"picture"
        );
        // Every file of the folder moved on, one by one, off the volume.
        let other = fresh(std::env::temp_dir(), "ntfs-folder");
        let file = target.join("inner").join("p.jpg");
        let away = other.join("p.jpg");
        move_one(&file, &away);
        assert_eq!(std::fs::read(&away).unwrap(), b"picture");
        assert_clean(&target.join("inner"));
        std::fs::remove_dir_all(other).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
#[ignore = "needs a FAT32 (SE_TEST_FAT32_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory on Windows; Suite crates-task"]
fn removable_task_read_only_file_moves_off_a_fat_volume() {
    // A move to another volume copies, then deletes the quarantined source
    // through its held handle: FAT has no FileDispositionInfoEx, and the
    // plain disposition refuses a read-only file unless the attribute is
    // lifted on that handle first (U17).
    let other = fresh(std::env::temp_dir(), "ntfs-readonly");
    for (index, dir) in removable_dirs("readonly").into_iter().enumerate() {
        for (name, target) in [
            ("r.txt", dir.join("within.txt")),
            ("s.txt", other.join(format!("away{index}.txt"))),
        ] {
            let source = dir.join(name);
            std::fs::write(&source, b"read only").unwrap();
            let mut permissions = std::fs::metadata(&source).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&source, permissions).unwrap();
            move_one(&source, &target);
            let metadata = std::fs::metadata(&target).unwrap();
            assert!(
                metadata.permissions().readonly(),
                "the attribute moves along"
            );
            assert_eq!(std::fs::read(&target).unwrap(), b"read only");
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            std::fs::set_permissions(&target, permissions).unwrap();
        }
        assert_clean(&dir);
        std::fs::remove_dir_all(dir).unwrap();
    }
    std::fs::remove_dir_all(other).unwrap();
}
