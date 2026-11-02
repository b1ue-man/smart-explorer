//! The sync walk resolves a local side's volume once, not per folder and file
//! (Windows only). Moved here from source_media/os/windows_plain_tests.rs: the walk
//! belongs to the bisync layer, and the switch and counters are the platform
//! hooks of the storage layer, reached through test-support (plan docs/plaene/
//! 2026-11-01-idealzustand, D13).
use crate::source_media::platform::plain::hooks;
use std::path::Path;

/// Resets the switch even when the walk panics.
struct Always;

impl Always {
    fn on() -> Self {
        hooks::resolve_always(true);
        Always
    }
}

impl Drop for Always {
    fn drop(&mut self) {
        hooks::resolve_always(false);
    }
}

/// `g{0..5}/f{0..8}/n{0..2}.dat` below `root`: 46 folders, 80 files.
fn sync_tree(root: &Path) {
    for group in 0..5 {
        for folder in 0..8 {
            let directory = root.join(format!("g{group}")).join(format!("f{folder}"));
            std::fs::create_dir_all(&directory).unwrap();
            for number in 0..2 {
                let text = format!("{group}-{folder}-{number}");
                std::fs::write(directory.join(format!("n{number}.dat")), text).unwrap();
            }
        }
    }
}

fn walk(root: &str, hash: crate::bisync::HashMode) -> crate::bisync::Tree {
    let backend = crate::vfs::LocalBackend::new(root);
    let globs = crate::bisync::empty_globset();
    let filter = crate::bisync::WalkFilter::basic(true, &globs);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    crate::bisync::walk_files(&backend, root, &cancel, &filter, hash, None).unwrap()
}

/// The sync walk of a local side, the path of `se sync compare` and of its
/// checksum mode: every folder's status and listing and every file read probe
/// their path. Resolved in full they cost one volume resolution each; now the
/// root's answer serves the tree. The walk's result is the same.
#[cfg(windows)]
#[test]
fn abgleich_task_sync_walk_resolves_the_volume_once_not_per_folder_and_file() {
    use crate::bisync::HashMode;
    let dir = tempfile::tempdir().unwrap();
    sync_tree(dir.path());
    let root = dir.path().to_string_lossy().replace('\\', "/");
    for (hash, per_folder, per_file) in [(HashMode::None, 2, 0), (HashMode::FullFresh, 2, 5)] {
        let watch = hooks::watch(dir.path());
        let before = {
            let _always = Always::on();
            walk(&root, hash)
        };
        let full = watch.count();
        drop(watch);
        let watch = hooks::watch(dir.path());
        let now = walk(&root, hash);
        let derived = watch.count();
        assert_eq!(now, before, "{hash:?}: the same tree");
        assert_eq!(now.len(), 80, "{hash:?}");
        if hash == HashMode::FullFresh {
            assert!(now.values().all(|sig| sig.hash != 0), "checksums read");
        }
        assert!(
            full >= 46 * per_folder + 80 * per_file,
            "{hash:?}: {full} full probes before"
        );
        // The root once; on a runner slow enough that the kept answers lapse
        // during the walk (`probe_cache::FRESH`, 2 s) once more per group
        // folder and the root: at most 8, against at least 92 before.
        assert!(
            derived <= 8,
            "{hash:?}: {derived} full probes now, {full} before"
        );
    }
}
