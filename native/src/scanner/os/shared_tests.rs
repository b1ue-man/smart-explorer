use super::*;

#[cfg(unix)]
#[test]
fn recursive_collection_does_not_follow_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let base = std::env::temp_dir().join(format!("se_scanner_link_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("real/sub")).unwrap();
    std::fs::write(base.join("real/sub/file.txt"), b"data").unwrap();
    symlink(base.join("real"), base.join("link")).unwrap();

    let outcome = collect_recursive(&base, false, 1, &AtomicBool::new(false));
    assert!(outcome.is_complete());
    let link = outcome
        .entries
        .iter()
        .find(|entry| entry.name.as_ref() == "link")
        .expect("symlink entry");
    assert!(link.is_symlink);
    assert!(!outcome
        .entries
        .iter()
        .any(|entry| entry.path.contains("/link/sub")));
    assert!(outcome
        .entries
        .iter()
        .any(|entry| entry.path.contains("/real/sub/file.txt")));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn recursive_collection_honors_preexisting_cancellation() {
    let cancel = AtomicBool::new(true);
    let outcome = collect_recursive(Path::new("."), false, 1, &cancel);
    assert!(outcome.canceled);
    assert!(outcome.entries.is_empty());
}

#[cfg(unix)]
#[test]
fn recursive_collection_rejects_a_link_like_root() {
    use std::os::unix::fs::symlink;

    let base = std::env::temp_dir().join(format!("se_scanner_root_link_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("real")).unwrap();
    symlink(base.join("real"), base.join("link")).unwrap();
    let outcome = collect_recursive(&base.join("link"), false, 1, &AtomicBool::new(false));
    assert!(!outcome.is_complete());
    assert!(outcome.entries.is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn recursive_collection_preserves_backslash_filename() {
    let base = std::env::temp_dir().join(format!("se_scanner_backslash_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("a\\b.txt"), b"data").unwrap();
    let outcome = collect_recursive(&base, false, 1, &AtomicBool::new(false));
    assert!(outcome.is_complete());
    let entry = outcome
        .entries
        .iter()
        .find(|entry| entry.name.as_ref() == "a\\b.txt")
        .expect("backslash filename");
    assert!(entry.path.ends_with("/a\\b.txt"));
    assert_eq!(std::fs::read(entry.path.as_ref()).unwrap(), b"data");
    let _ = std::fs::remove_dir_all(&base);
}
