//! The cleartext map of a vault storage (stage `vault_compare`): folders
//! and entries with their cleartext paths, shortened names, and the end of
//! the walk by a cancel flag or a lock.
use std::sync::atomic::AtomicBool;

use super::compare_test_support::Fx;
use super::dir_map::DirMap;
use crate::vault::core::listing::NodeShape;
use crate::vfs::LocalBackend;

fn build(fx: &Fx, root: &str) -> std::io::Result<DirMap> {
    DirMap::build(
        &LocalBackend::new("/"),
        root,
        &fx.session,
        &AtomicBool::new(false),
    )
}

#[test]
fn vault_task_dir_map_maps_paths_shapes_and_long_names() {
    let fx = Fx::bare();
    let long = format!("/{}.txt", "q".repeat(170));
    fx.write(&fx.root_a, "/Bericht.txt", b"b");
    fx.write(&fx.root_a, "/Ordner/Tief/x.bin", b"x");
    fx.write(&fx.root_a, &long, b"lang");
    let map = build(&fx, &fx.root_a).expect("map");
    // Every folder has a content directory with its path and ID.
    let folders: Vec<&str> = map.dirs.values().map(|dir| dir.clear.as_str()).collect();
    for path in ["/", "/Ordner", "/Ordner/Tief"] {
        assert!(folders.contains(&path), "{folders:?}");
        let content = map.by_clear.get(path).expect(path);
        assert!(content.starts_with("d/"), "{content}");
        assert_eq!(map.dirs[content].clear, path);
    }
    assert_eq!(map.dirs[&map.by_clear["/"]].id, "", "the root has no ID");
    let entry = |path: &str| {
        map.names
            .values()
            .find(|info| info.clear == path)
            .unwrap_or_else(|| panic!("no entry {path}"))
    };
    assert_eq!(entry("/Bericht.txt").shape, NodeShape::File);
    assert!(!entry("/Bericht.txt").shortened);
    assert_eq!(entry("/Ordner").shape, NodeShape::Dir);
    assert!(entry("/Ordner").child.is_some());
    assert_eq!(entry("/Ordner/Tief/x.bin").shape, NodeShape::File);
    // A name longer than the threshold is stored as `.c9s` and still mapped.
    let shortened = entry(&long);
    assert!(shortened.shortened && shortened.encrypted.ends_with(".c9r"));
    assert_eq!(shortened.shape, NodeShape::File);
    assert!(map.findings.is_empty(), "{:?}", map.findings);
}

#[test]
fn vault_task_dir_map_walk_ends_with_cancel_or_lock() {
    let fx = Fx::bare();
    fx.write(&fx.root_a, "/Ordner/x.txt", b"x");
    let cancel = AtomicBool::new(true);
    let canceled = DirMap::build(&LocalBackend::new("/"), &fx.root_a, &fx.session, &cancel)
        .err()
        .expect("canceled");
    assert_eq!(canceled.kind(), std::io::ErrorKind::Interrupted);
    // After the lock the key is gone: the walk answers `vault_locked`.
    fx.session.lock();
    let locked = build(&fx, &fx.root_a).err().expect("locked");
    assert!(
        crate::vault::core::locked_site(&locked).is_some(),
        "{locked}"
    );
}
