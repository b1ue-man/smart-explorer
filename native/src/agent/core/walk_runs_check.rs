//! Test check of the folder-run order of a walk with listing attributes
//! (`HashWalkRequest::meta`): every item comes after its folder's entry, and
//! the items of one folder (its entries and omissions) form one unbroken run.
//! A folder's own omission (it could not be opened, or its listing broke off)
//! belongs to that folder and may come in its parent's run or in its own.
use crate::vfs::HashWalkItem;
use std::collections::HashSet;

/// Panics at the first item outside this order; returns the number of runs.
pub(crate) fn assert_folder_runs(items: &[HashWalkItem]) -> usize {
    let mut folders: HashSet<String> = HashSet::from([String::new()]);
    let mut finished: HashSet<String> = HashSet::new();
    let mut current: Option<String> = None;
    let mut runs = 0;
    for (index, item) in items.iter().enumerate() {
        let (rel, own_failure) = match item {
            HashWalkItem::Entry(entry) => (entry.rel.as_str(), false),
            HashWalkItem::Omitted(omission) => (
                omission.rel.as_str(),
                folders.contains(&omission.rel) && !finished.contains(&omission.rel),
            ),
        };
        if rel.is_empty() || own_failure {
            continue;
        }
        let folder = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
        assert!(
            folders.contains(folder),
            "item {index} ({rel}) arrived before its folder"
        );
        if current.as_deref() != Some(folder) {
            if let Some(done) = current.take() {
                finished.insert(done);
            }
            assert!(
                !finished.contains(folder),
                "the run of {folder:?} resumed at item {index} ({rel})"
            );
            current = Some(folder.to_string());
            runs += 1;
        }
        if let HashWalkItem::Entry(entry) = item {
            if entry.is_dir {
                folders.insert(entry.rel.clone());
            }
        }
    }
    runs
}

/// Folders whose entries and omissions (names that are no UTF-8) alternate,
/// many small folders for the host's threads, and one folder larger than
/// several portions of the walk.
pub(crate) fn run_tree(root: &std::path::Path) {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let big = root.join("big");
    std::fs::create_dir_all(&big).unwrap();
    for index in 0..3000 {
        let name = format!("{index:04}-{}", "n".repeat(90));
        std::fs::write(big.join(name), b"").unwrap();
    }
    for index in 0..5u8 {
        let name = [b'x', 0xff, b'0' + index];
        std::fs::write(big.join(OsStr::from_bytes(&name)), b"").unwrap();
    }
    for folder in 0..40 {
        let outer = root.join(format!("d{folder}"));
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        for file in 0..3 {
            std::fs::write(inner.join(format!("f{file}.txt")), b"12345").unwrap();
        }
        std::fs::write(outer.join(OsStr::from_bytes(&[b'y', 0xfe])), b"").unwrap();
    }
}
