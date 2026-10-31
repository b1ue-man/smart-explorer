//! Folder index walk: the device lock only on a protected subtree, and the same
//! folders as before. Test names start with `abgleich_task_` (the remote task
//! suite selects them).
use super::build_index_excluding;
use crate::folder_index::{FolderIndex, IndexMsg};
use crate::source_media::{test_profiles, Source};
use crossbeam_channel::unbounded;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

/// A temporary tree, removed when dropped.
struct Tree(PathBuf);

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `plain/{a.txt, deep/b.txt, many/fNN/x.txt}` and `optical/{c.txt, sub/d.txt}`
/// below a fresh root. Returns the tree and every folder below the root, relative
/// to it; the root itself is the empty path.
fn tree(label: &str) -> (Tree, BTreeSet<String>) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("se_index_{label}_{}_{nanos}", std::process::id()));
    let tree = Tree(root.clone());
    let mut files = vec![
        "plain/a.txt".to_string(),
        "plain/deep/b.txt".to_string(),
        "optical/c.txt".to_string(),
        "optical/sub/d.txt".to_string(),
    ];
    for number in 0..40 {
        files.push(format!("plain/many/f{number:02}/x.txt"));
    }
    let mut folders = BTreeSet::from([String::new()]);
    for rel in &files {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"x").unwrap();
        let parts: Vec<&str> = rel.split('/').collect();
        let mut prefix = String::new();
        for part in &parts[..parts.len() - 1] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            folders.insert(prefix.clone());
        }
    }
    (tree, folders)
}

/// Every folder of `index` below `root`, relative to it.
fn relative_folders(index: &FolderIndex, root: &str) -> BTreeSet<String> {
    index
        .iter()
        .map(|path| {
            let path: &str = path;
            path.strip_prefix(root)
                .unwrap_or(path)
                .trim_start_matches('/')
                .to_string()
        })
        .collect()
}

/// The index of `root` built by the walk under test.
fn index_of(root: &Path) -> FolderIndex {
    let (tx, _rx) = unbounded::<IndexMsg>();
    let cancel = AtomicBool::new(false);
    let Ok(index) = build_index_excluding(vec![root.to_path_buf()], &tx, &cancel, &[]) else {
        panic!("the folder index walk failed");
    };
    index
}

#[test]
fn abgleich_task_folder_index_takes_the_guard_only_on_the_protected_subtree() {
    let (tree, folders) = tree("guard");
    let root_text = tree.0.to_string_lossy().into_owned();
    let optical = tree.0.join("optical");
    let device = format!("abgleich-task-index-{}", tree.0.display());
    let _profile = test_profiles::register(
        &optical,
        Source {
            device: device.clone(),
            generation: "abgleich-task".to_string(),
            read_only: true,
        },
    );
    let plain_watch = test_profiles::watch_probes(&tree.0.join("plain"));
    let optical_watch = test_profiles::watch_probes(&optical);
    let index = index_of(&tree.0);
    assert_eq!(
        relative_folders(&index, &root_text),
        folders,
        "the index holds every folder, the protected ones included"
    );
    assert_eq!(
        plain_watch.count(),
        0,
        "no device probe below the plain folders"
    );
    assert!(
        optical_watch.count() >= 1,
        "the protected subtree was determined"
    );
    assert!(
        test_profiles::acquisitions(&device) >= 2,
        "the protected listings and entries took the device lock"
    );
}

#[test]
fn abgleich_task_folder_index_asks_for_its_device_once_not_per_folder() {
    let (tree, folders) = tree("probes");
    let root_text = tree.0.to_string_lossy().into_owned();
    let watch = test_profiles::watch_probes(&tree.0);
    let index = index_of(&tree.0);
    assert_eq!(
        relative_folders(&index, &root_text),
        folders,
        "the index holds every folder"
    );
    assert!(
        watch.count() <= 3,
        "device probes below the root: {} for {} folders (the folders are not probed)",
        watch.count(),
        folders.len()
    );
}
