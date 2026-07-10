use super::model::{FolderIndex, IndexMsg, MAX_INDEX_FILE_BYTES};
use super::search::fuzzy_score;
#[cfg(unix)]
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

fn s(s: &str) -> i32 {
    fuzzy_score(s.to_lowercase().as_bytes(), s.as_bytes()).unwrap_or(0)
}

#[test]
fn basic() {
    // Identical match scores higher than substring
    assert!(s("Downloads") > 0);
    // "dnlds" matches "Downloads" but lower than "downloads"
    let exact = fuzzy_score(b"downloads", b"C:/Users/Silas/Downloads".as_ref()).unwrap();
    let fuzzy = fuzzy_score(b"dnlds", b"C:/Users/Silas/Downloads".as_ref()).unwrap();
    assert!(exact > fuzzy);
}

#[test]
fn no_match() {
    assert!(fuzzy_score(b"xyz", b"abc").is_none());
}

#[test]
fn bounded_index_rejects_unpersistable_paths() {
    let mut index = FolderIndex::new();
    assert!(index
        .try_insert("/home/user/Documents".to_string())
        .unwrap());
    assert!(!index
        .try_insert("/home/user/Documents".to_string())
        .unwrap());
    assert!(index
        .try_insert("/home/user/bad\npath".to_string())
        .is_err());
}

#[test]
fn save_and_load_stream_a_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("folder_index.txt");
    let mut index = FolderIndex::new();
    index
        .try_insert("/home/user/Documents".to_string())
        .unwrap();
    index.try_insert("/home/user/Pictures".to_string()).unwrap();
    index.save(&target).unwrap();

    let loaded = FolderIndex::load(&target).unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(loaded.iter().any(|path| path == "/home/user/Documents"));
}

#[test]
fn load_rejects_an_oversized_sparse_file() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("folder_index.txt");
    let file = std::fs::File::create(&target).unwrap();
    file.set_len(MAX_INDEX_FILE_BYTES + 1).unwrap();
    assert!(FolderIndex::load(&target).is_err());
}

#[test]
fn canceled_build_does_not_publish_or_persist_partial_data() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let target = temp.path().join("folder_index.txt");
    let cancel = Arc::new(AtomicBool::new(true));
    let (tx, rx) = crossbeam_channel::unbounded();
    FolderIndex::build_async(vec![root], target.clone(), tx, cancel).unwrap();

    assert!(matches!(terminal_message(&rx), IndexMsg::Canceled));
    assert!(!target.exists());
}

#[test]
fn invalid_root_fails_without_replacing_the_index() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("missing");
    let target = temp.path().join("folder_index.txt");
    std::fs::write(&target, "/existing/index\n").unwrap();
    let original = std::fs::read(&target).unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    FolderIndex::build_async(
        vec![root],
        target.clone(),
        tx,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();

    assert!(matches!(terminal_message(&rx), IndexMsg::Failed(_)));
    assert_eq!(std::fs::read(target).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn traversal_does_not_follow_symlinked_directories() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let real = root.join("real");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(outside.join("must-not-appear")).unwrap();
    symlink(&outside, root.join("linked")).unwrap();
    let target = temp.path().join("folder_index.txt");
    let (tx, rx) = crossbeam_channel::unbounded();
    FolderIndex::build_async(
        vec![root.clone()],
        target.clone(),
        tx,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();

    let IndexMsg::Complete(index) = terminal_message(&rx) else {
        panic!("expected a complete folder index");
    };
    let real = normalized(&real);
    assert!(index.iter().any(|path| path == &real));
    assert!(!index.iter().any(|path| path.contains("linked")));
    assert!(target.exists());
}

#[cfg(unix)]
#[test]
fn excessive_depth_fails_without_persisting_partial_data() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let mut current = root.clone();
    for _ in 0..513 {
        current.push("d");
        std::fs::create_dir(&current).unwrap();
    }
    let target = temp.path().join("folder_index.txt");
    let (tx, rx) = crossbeam_channel::unbounded();
    FolderIndex::build_async(
        vec![root],
        target.clone(),
        tx,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();

    assert!(matches!(terminal_message(&rx), IndexMsg::Failed(_)));
    assert!(!target.exists());
}

fn terminal_message(rx: &crossbeam_channel::Receiver<IndexMsg>) -> IndexMsg {
    loop {
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            IndexMsg::Progress { .. } => {}
            terminal => return terminal,
        }
    }
}

#[cfg(unix)]
fn normalized(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
