//! The preview "Zielordner entscheiden": five counts from one listing per
//! folder, over many folders at once.
use super::super::super::test_backend::{job, write};
use super::count_folder_conflict;
use crate::transfer::{folder_conflicts, Endpoint};
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, UNIX_EPOCH};

fn file(path: &Path, bytes: &[u8], seconds: u64) {
    write(path, bytes);
    let handle = fs::OpenOptions::new().write(true).open(path).expect("open");
    handle
        .set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
        .expect("set time");
}

fn counted(source: &Path, dest: &Path) -> crate::transfer::FolderConflictPreview {
    let transfer = job(Endpoint::Local, Endpoint::Local, dest, &[source]);
    let stop = AtomicBool::new(false);
    let mut folders = folder_conflicts(&transfer, &stop).expect("roots");
    assert_eq!(folders.len(), 1, "the destination folder exists");
    let mut folder = folders.remove(0);
    count_folder_conflict(&transfer, &mut folder, &stop, &|_| {});
    assert!(folder.counted && folder.complete, "{folder:?}");
    folder
}

#[test]
fn abgleich_task_preview_counts_new_newer_equal_target_newer_and_conflict() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("local/src");
    let dest = root.path().join("dest");
    file(&source.join("new.txt"), b"new", 5_000);
    file(&source.join("sub/deep.txt"), b"deep", 5_000);
    file(&source.join("sub/also/deeper.txt"), b"deeper", 5_000);
    file(&source.join("up.txt"), b"newer content", 6_000);
    file(&source.join("same.txt"), b"same", 5_000);
    file(&source.join("keep.txt"), b"from source", 1_000);
    file(&source.join("clash.txt"), b"abc", 2_000);
    file(&dest.join("src/up.txt"), b"old", 1_000);
    file(&dest.join("src/same.txt"), b"same", 5_000);
    file(&dest.join("src/keep.txt"), b"destination is newer", 9_000);
    file(&dest.join("src/clash.txt"), b"abcde", 2_000);
    let folder = counted(&source, &dest);
    assert_eq!(folder.new_files, 3, "new.txt and the two below sub");
    assert_eq!(folder.newer_files, 1);
    assert_eq!(folder.equal_files, 1);
    assert_eq!(folder.target_newer_files, 1);
    assert_eq!(folder.conflict_files, 1);
    assert_eq!(folder.colliding_files, 4, "the same-named ones together");
    assert!(!folder.samples.is_empty() && folder.samples.len() <= 5);
    assert_eq!(folder.omitted, 0);
}

#[test]
fn abgleich_task_preview_counts_a_wide_tree_exactly_and_without_target_folders() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("local/src");
    let dest = root.path().join("dest");
    fs::create_dir_all(dest.join("src")).expect("destination");
    for folder in 0..40 {
        for name in 0..5 {
            file(&source.join(format!("d{folder}/f{name}.txt")), b"x", 5_000);
        }
    }
    // Half of the folders exist at the target with equal files.
    for folder in 0..20 {
        for name in 0..5 {
            file(
                &dest.join(format!("src/d{folder}/f{name}.txt")),
                b"x",
                5_000,
            );
        }
    }
    let folder = counted(&source, &dest);
    assert_eq!(folder.equal_files, 100);
    assert_eq!(folder.new_files, 100, "the other folders are absent");
    assert_eq!(folder.colliding_files, 100);
    assert_eq!(
        folder.newer_files + folder.target_newer_files + folder.conflict_files,
        0
    );
    assert!(
        !dest.join("src/d30").exists(),
        "the preview never creates anything"
    );
}
