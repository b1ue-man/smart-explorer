//! `hidden` in the walk of a local sync side (`local_keyed`): on Windows the
//! file attribute decides and a dot name is an ordinary name; elsewhere a
//! leading dot hides, as the agent and SFTP list it. Keys-only walks
//! (`Collected::keys_of`) name the same folders as the walk with runs.
use super::{local_keyed, Collected, WireItem};
use crate::agent_proto::digest;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

/// Name to `hidden` of every entry of the root folder's run.
fn hidden_of_root(root: &std::path::Path) -> BTreeMap<String, bool> {
    let cancel = AtomicBool::new(false);
    let collected = local_keyed(root.to_str().expect("utf-8 path"), digest::NONE, &cancel)
        .expect("local keyed walk");
    let (_, items) = collected
        .runs
        .into_iter()
        .find(|(rel, _)| rel.is_empty())
        .expect("the root folder has a run");
    items
        .into_iter()
        .filter_map(|item| match item {
            WireItem::Entry(entry) => Some((entry.rel, entry.meta.expect("meta").hidden)),
            WireItem::Omitted(_) => None,
        })
        .collect()
}

#[cfg(windows)]
fn hide(path: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
    let set = unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN) };
    assert_ne!(set, 0, "{}", std::io::Error::last_os_error());
}

#[cfg(windows)]
#[test]
fn abgleich_task_windows_local_walk_takes_hidden_from_the_file_attribute() {
    let root = tempfile::tempdir().expect("temp dir");
    std::fs::write(root.path().join(".dotted.txt"), b"dot name, no attribute").unwrap();
    std::fs::write(root.path().join("attributed.txt"), b"attribute, no dot").unwrap();
    std::fs::write(root.path().join("plain.txt"), b"neither").unwrap();
    hide(&root.path().join("attributed.txt"));
    let hidden = hidden_of_root(root.path());
    assert!(!hidden[".dotted.txt"], "a dot name is no attribute");
    assert!(hidden["attributed.txt"], "the attribute hides");
    assert!(!hidden["plain.txt"]);
}

#[cfg(not(windows))]
#[test]
fn abgleich_task_local_walk_keeps_dot_names_hidden_where_there_is_no_attribute() {
    let root = tempfile::tempdir().expect("temp dir");
    std::fs::write(root.path().join(".dotted.txt"), b"dot").unwrap();
    std::fs::write(root.path().join("plain.txt"), b"plain").unwrap();
    let hidden = hidden_of_root(root.path());
    assert!(hidden[".dotted.txt"]);
    assert!(!hidden["plain.txt"]);
}

#[test]
fn abgleich_task_keys_only_walk_names_the_folders_of_the_walk_with_runs() {
    let root = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(root.path().join("a/b")).unwrap();
    std::fs::create_dir_all(root.path().join("c")).unwrap();
    std::fs::write(root.path().join("a/b/f.txt"), b"x").unwrap();
    std::fs::write(root.path().join("c/g.txt"), b"y").unwrap();
    let path = root.path().to_str().expect("utf-8 path");
    let cancel = AtomicBool::new(false);
    let full = local_keyed(path, digest::NONE, &cancel).expect("walk with runs");
    let mut full_keys = full.keys;
    let mut only: Vec<(String, [u8; 32])> = Collected::keys_of(path, digest::NONE, &cancel)
        .expect("keys only")
        .into_iter()
        .map(|(rel, key, _)| (rel, key))
        .collect();
    full_keys.sort();
    only.sort();
    assert_eq!(only, full_keys, "the same folders carry the same keys");
    let folders: Vec<&str> = only.iter().map(|(rel, _)| rel.as_str()).collect();
    assert_eq!(folders, ["", "a", "a/b", "c"]);
}
