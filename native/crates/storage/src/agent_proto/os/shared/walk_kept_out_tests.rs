//! The folder that holds the digest cache is a protected omission of the
//! agent's walks (`walk_kept_out.rs`, `hash_cache::is_cache_folder`): its row
//! is listed, its content is not walked, and an omission of the same path
//! tells the sync that its absence on the other side is not a deletion. The
//! cache is one per process: these tests take `hash_cache::test_turn`.
use super::hash_cache;
use super::{digest, omission, KeyedWireItem, LocalKeyed};
use std::path::Path;
use std::sync::atomic::AtomicBool;

fn write(path: &Path, content: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// A home folder as a server has it: a file of the user and the agent's
/// cache folder with a cache file in it, below the walked root.
fn home(root: &Path) -> std::path::PathBuf {
    write(&root.join("home/notes.txt"), b"abc");
    let cache = root.join("home/.cache/se-agent");
    write(&cache.join("hash-cache-v1.bin"), b"cache");
    write(&cache.join("hash-cache-v1.lock"), b"");
    cache
}

fn walked(root: &Path) -> LocalKeyed {
    super::local_keyed(root.to_str().unwrap(), digest::MD5, &AtomicBool::new(false)).unwrap()
}

/// The entries (`rel`, `is_dir`) and the omissions (`rel`, `reason`).
type Items = (Vec<(String, bool)>, Vec<(String, u8)>);

/// The entries and omissions of a walk.
fn items(collected: &LocalKeyed) -> Items {
    let (mut entries, mut omissions) = (Vec::new(), Vec::new());
    for (_, run) in &collected.runs {
        for item in run {
            match item {
                KeyedWireItem::Entry(entry) => entries.push((entry.rel.clone(), entry.is_dir)),
                KeyedWireItem::Omitted(omitted) => {
                    omissions.push((omitted.rel.clone(), omitted.reason))
                }
            }
        }
    }
    (entries, omissions)
}

#[test]
fn abgleich_task_walk_lists_the_cache_folder_as_a_protected_omission() {
    let cases: [(&str, fn()); 2] = [
        (
            "abgleich_task_walk_lists_the_cache_folder_as_a_protected_omission",
            || {
                let _turn = hash_cache::test_turn();
                let data = tempfile::tempdir().unwrap();
                let cache = home(data.path());
                hash_cache::configure(Some(cache));
                let (entries, omissions) = items(&walked(data.path()));
                let folder = "home/.cache/se-agent";
                assert!(
                    entries.contains(&(folder.to_string(), true)),
                    "the folder is a row, as its parent's listing shows it: {entries:?}"
                );
                assert!(
                    omissions.contains(&(folder.to_string(), omission::UNREADABLE)),
                    "and an omission of the same path: {omissions:?}"
                );
                assert!(
                    entries
                        .iter()
                        .all(|(rel, _)| !rel.starts_with("home/.cache/se-agent/")),
                    "nothing inside it was walked: {entries:?}"
                );
                assert!(entries.contains(&("home/notes.txt".to_string(), false)));
                hash_cache::configure(None);
            },
        ),
        (
            "abgleich_task_walk_walks_a_folder_of_the_same_name_that_is_not_the_cache_folder",
            || {
                let _turn = hash_cache::test_turn();
                let data = tempfile::tempdir().unwrap();
                let cache = home(data.path());
                write(&data.path().join("other/.cache/se-agent/keep.txt"), b"abc");
                hash_cache::configure(Some(cache));
                let (entries, omissions) = items(&walked(data.path()));
                assert!(entries.contains(&("other/.cache/se-agent/keep.txt".to_string(), false)));
                assert!(
                    omissions
                        .iter()
                        .all(|(rel, _)| rel != "other/.cache/se-agent"),
                    "{omissions:?}"
                );
                hash_cache::configure(None);
            },
        ),
    ];
    for (case, check) in cases {
        std::panic::catch_unwind(check).unwrap_or_else(|_| panic!("{case}"));
    }
}

#[test]
fn abgleich_task_the_cache_folder_is_found_under_another_spelling_only() {
    let _turn = hash_cache::test_turn();
    let data = tempfile::tempdir().unwrap();
    let cache = home(data.path());
    hash_cache::configure(Some(cache.clone()));
    assert!(hash_cache::is_cache_folder(&cache));
    // The same folder reached through `..`.
    let respelled = data.path().join("home/.cache/../.cache/se-agent");
    assert!(hash_cache::is_cache_folder(&respelled));
    // Its parent, a sibling and a folder of its name elsewhere are not it.
    assert!(!hash_cache::is_cache_folder(cache.parent().unwrap()));
    write(&data.path().join("home/.cache/sibling/x"), b"x");
    assert!(!hash_cache::is_cache_folder(
        &data.path().join("home/.cache/sibling")
    ));
    write(&data.path().join("elsewhere/se-agent/x"), b"x");
    assert!(!hash_cache::is_cache_folder(
        &data.path().join("elsewhere/se-agent")
    ));
    // A process without a cache folder leaves nothing out.
    hash_cache::configure(None);
    assert!(!hash_cache::is_cache_folder(&cache));
}
