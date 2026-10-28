//! Paste onto removable media: FAT32 and exFAT volumes, into their root and
//! into a folder, as the explorer's paste job does (tree, keep both). The
//! suite attaches real volumes and names their roots in
//! `SE_TASK_REMOVABLE_ROOTS` (`;`-separated, e.g. `Y:\;Z:\`).
use super::test_backend::{job, run, write};
use crate::transfer::Endpoint;
use crate::types::CopyMode;
use std::fs;
use std::path::{Path, PathBuf};

fn roots() -> Vec<PathBuf> {
    std::env::var("SE_TASK_REMOVABLE_ROOTS")
        .unwrap_or_default()
        .split(';')
        .filter(|root| !root.trim().is_empty())
        .map(|root| PathBuf::from(root.trim()))
        .collect()
}

fn source(root: &Path) -> (PathBuf, PathBuf) {
    let folder = root.join("Vault");
    write(&folder.join("a.txt"), b"alpha");
    write(&folder.join("sub/b.txt"), b"beta");
    fs::create_dir_all(folder.join("empty")).expect("empty folder");
    let file = root.join("note.txt");
    write(&file, b"note");
    (folder, file)
}

fn paste(target: &Path, sources: &[&Path], mode: CopyMode) {
    let mut transfer = job(Endpoint::Local, Endpoint::Local, target, sources);
    transfer.mode = mode;
    let finished = run(transfer);
    assert!(!finished.canceled, "{}", target.display());
    assert!(
        finished.issues.is_empty() && finished.errors.is_empty(),
        "{}: {:?} {:?}",
        target.display(),
        finished.issues,
        finished.errors
    );
}

fn assert_tree(folder: &Path) {
    assert_eq!(fs::read(folder.join("a.txt")).expect("a"), b"alpha");
    assert_eq!(fs::read(folder.join("sub/b.txt")).expect("b"), b"beta");
    assert!(folder.join("empty").is_dir(), "{}", folder.display());
}

#[test]
fn removable_media_task_paste_into_root_and_folder() {
    let roots = roots();
    for root in &roots {
        let local = tempfile::tempdir().expect("temp dir");
        let (folder, file) = source(local.path());
        // Into the volume root, twice: the second paste keeps both.
        paste(root, &[&folder, &file], CopyMode::Copy);
        assert_tree(&root.join("Vault"));
        assert_eq!(fs::read(root.join("note.txt")).expect("note"), b"note");
        paste(root, &[&folder, &file], CopyMode::Copy);
        assert_tree(&root.join("Vault (2)"));
        assert_eq!(
            fs::read(root.join("note (2).txt")).expect("note 2"),
            b"note"
        );
        // Into a folder on the volume.
        let inner = root.join("Ordner");
        fs::create_dir(&inner).expect("folder on the volume");
        paste(&inner, &[&folder, &file], CopyMode::Copy);
        assert_tree(&inner.join("Vault"));
        assert_eq!(fs::read(inner.join("note.txt")).expect("note"), b"note");
        // Cut and paste into the root, next to the two earlier copies.
        let moved = tempfile::tempdir().expect("temp dir");
        let (folder, file) = source(moved.path());
        paste(root, &[&folder, &file], CopyMode::Move);
        assert_tree(&root.join("Vault (3)"));
        assert_eq!(
            fs::read(root.join("note (3).txt")).expect("note 3"),
            b"note"
        );
        assert!(!folder.exists() && !file.exists(), "move keeps no source");
        println!("removable media root checked: {}", root.display());
    }
    println!("removable media roots checked: {}", roots.len());
}
