//! Folder keys in the file rule "neuere ersetzen ältere" (copy and paste,
//! "Zusammenführen"): a source folder with the key of the same-named target
//! folder is equal as a whole and the target folder is not listed; the folders
//! with other keys take the way without keys, and every result (copies,
//! replacements, counts, messages, the preview's five counts) equals the way
//! without keys. A target without the keyed walk takes the way without keys.
//! (The source side of the same keys: `abgleich_task_source_keys_tests.rs`.)
use super::super::test_backend::{job, run, write, Fake, Finished};
use crate::transfer::{
    count_folder_conflict, folder_conflicts, Endpoint, FolderConflict, FolderConflictPolicy,
};
use crate::types::Conflict;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

pub(super) fn file(path: &Path, bytes: &[u8], seconds: u64) {
    write(path, bytes);
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open")
        .set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
        .expect("set time");
}

/// Source `local/src` and target `dest/src`: `a`, `b/deep` and `d/e` hold
/// the same files on both sides (equal keys); `c` and `d` differ.
pub(super) fn trees(root: &Path) -> (PathBuf, PathBuf) {
    let source = root.join("local/src");
    let dest = root.join("dest");
    for base in [&source, &dest.join("src")] {
        file(&base.join("a/one.txt"), b"one", 5_000);
        file(&base.join("a/two.txt"), b"two!", 5_000);
        file(&base.join("b/deep/x.txt"), b"deep", 5_000);
        file(&base.join("d/e/same.txt"), b"same", 5_000);
    }
    file(&source.join("c/new.txt"), b"newer content", 6_000);
    file(&source.join("c/fresh.txt"), b"fresh", 5_000);
    file(&source.join("d/one.txt"), b"source one, longer", 7_000);
    file(&dest.join("src/c/new.txt"), b"old", 1_000);
    file(&dest.join("src/d/one.txt"), b"one", 1_000);
    (source, dest)
}

pub(super) fn files_under(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                let rel = path.strip_prefix(base).expect("below base");
                out.insert(
                    rel.to_string_lossy().replace('\\', "/"),
                    fs::read(&path).expect("read"),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

struct Merged {
    finished: Finished,
    listings: usize,
    key_walks: usize,
    tree: BTreeMap<String, Vec<u8>>,
}

pub(super) fn merging() -> FolderConflictPolicy {
    FolderConflictPolicy::new(FolderConflict::Merge)
}

/// Uploads `source` into `dest` under the file rule, the target answering
/// the keyed walk or not.
fn merge(source: &Path, dest: &Path, keyed: bool) -> Merged {
    let mut fake = Fake::new("keys-merge");
    fake.replaces = true;
    fake.faults.keyed = keyed;
    let (fake, handle) = fake.handle();
    let mut transfer = job(Endpoint::Local, Endpoint::Remote(handle), dest, &[source]);
    transfer.folders = merging();
    transfer.conflict = Conflict::Update;
    let finished = run(transfer);
    Merged {
        finished,
        listings: fake.counters.listings.load(Ordering::SeqCst),
        key_walks: fake.counters.key_walks.load(Ordering::SeqCst),
        tree: files_under(dest),
    }
}

/// The numbers a surface shows for a finished merge.
pub(super) fn counts(finished: &Finished) -> [u64; 7] {
    let progress = &finished.progress;
    [
        progress.replaced,
        progress.skipped_equal,
        progress.kept_newer,
        progress.kept_both,
        progress.skipped,
        progress.bytes_done,
        progress.bytes_total,
    ]
}

#[test]
fn abgleich_task_equal_subtrees_are_not_listed_and_the_rest_decides_as_before() {
    let with = tempfile::tempdir().expect("temp dir");
    let without = tempfile::tempdir().expect("temp dir");
    let (source, dest) = trees(with.path());
    let keyed = merge(&source, &dest, true);
    let (source_plain, dest_plain) = trees(without.path());
    let plain = merge(&source_plain, &dest_plain, false);

    assert!(
        keyed.finished.issues.is_empty(),
        "{:?}",
        keyed.finished.issues
    );
    assert!(
        plain.finished.issues.is_empty(),
        "{:?}",
        plain.finished.issues
    );
    assert_eq!(keyed.finished.progress.replaced, 2, "c/new.txt, d/one.txt");
    assert_eq!(
        keyed.finished.progress.skipped_equal, 4,
        "a/one, a/two, b/deep/x, d/e/same"
    );
    assert_eq!(counts(&keyed.finished), counts(&plain.finished));
    assert_eq!(keyed.tree, plain.tree, "the same files, the same bytes");
    assert_eq!(keyed.tree["src/c/new.txt"], b"newer content");
    assert_eq!(keyed.tree["src/c/fresh.txt"], b"fresh");
    assert_eq!(keyed.tree["src/d/one.txt"], b"source one, longer");

    assert_eq!(keyed.key_walks, 1, "the target is keyed once");
    assert_eq!(
        keyed.listings, 2,
        "only c and d are listed: the folders a, b/deep and d/e have equal keys"
    );
    assert_eq!(plain.key_walks, 0, "a target without the keyed walk");
    assert_eq!(
        plain.listings, 5,
        "without keys each folder with files is listed: a, b/deep, c, d, d/e"
    );
}

#[test]
fn abgleich_task_an_equal_tree_is_settled_by_one_key_without_any_listing() {
    let root = tempfile::tempdir().expect("temp dir");
    let (source, dest) = trees(root.path());
    // Make the whole tree equal: bring the target to the source.
    fs::remove_dir_all(dest.join("src/c")).expect("clean");
    fs::remove_file(dest.join("src/d/one.txt")).expect("clean");
    fs::remove_dir_all(source.join("c")).expect("clean");
    fs::remove_file(source.join("d/one.txt")).expect("clean");
    let before = files_under(&dest);
    let merged = merge(&source, &dest, true);
    assert!(
        merged.finished.issues.is_empty(),
        "{:?}",
        merged.finished.issues
    );
    assert_eq!(merged.listings, 0, "no folder of an equal tree is listed");
    assert_eq!(merged.key_walks, 1);
    assert_eq!(merged.finished.progress.skipped_equal, 4);
    assert_eq!(merged.finished.progress.replaced, 0);
    assert_eq!(merged.tree, before, "nothing written, nothing duplicated");
    let progress = &merged.finished.progress;
    assert_eq!(progress.bytes_done, progress.bytes_total);
}

#[test]
fn abgleich_task_a_file_with_an_unknown_time_never_takes_the_key_way() {
    // Both sides store the epoch: the keys are equal, but a time of 0 is a
    // time the rule does not know, so it keeps both, with and without keys.
    let results: Vec<Merged> = [true, false]
        .into_iter()
        .map(|keyed| {
            let root = tempfile::tempdir().expect("temp dir");
            let source = root.path().join("local/src");
            let dest = root.path().join("dest");
            file(&source.join("same.txt"), b"same", 0);
            file(&dest.join("src/same.txt"), b"same", 0);
            merge(&source, &dest, keyed)
        })
        .collect();
    for merged in &results {
        assert_eq!(merged.finished.progress.kept_both, 1);
        assert_eq!(merged.finished.progress.skipped_equal, 0);
        assert!(
            merged.tree.contains_key("src/same (2).txt"),
            "{:?}",
            merged.tree.keys()
        );
        assert!(merged.listings >= 1, "the file takes the listing");
    }
    assert_eq!(counts(&results[0].finished), counts(&results[1].finished));
}

/// The preview of the folder decision: the five counts of the keyed and the
/// plain way equal, and the keyed way lists the target folders of the
/// unequal subtrees only.
#[test]
fn abgleich_task_preview_counts_equal_folders_by_key_with_the_same_counts() {
    let preview = |keyed: bool| {
        let root = tempfile::tempdir().expect("temp dir");
        let (source, dest) = trees(root.path());
        let mut fake = Fake::new("keys-preview");
        fake.faults.keyed = keyed;
        let (fake, handle) = fake.handle();
        let mut transfer = job(Endpoint::Local, Endpoint::Remote(handle), &dest, &[&source]);
        transfer.conflict = Conflict::Update;
        let stop = AtomicBool::new(false);
        let mut folders = folder_conflicts(&transfer, &stop).expect("roots");
        assert_eq!(folders.len(), 1, "the destination folder exists");
        let mut folder = folders.remove(0);
        count_folder_conflict(&transfer, &mut folder, &stop, &|_| {});
        assert!(folder.counted && folder.complete, "{folder:?}");
        (
            folder,
            fake.counters.listings.load(Ordering::SeqCst),
            fake.counters.key_walks.load(Ordering::SeqCst),
        )
    };
    let (keyed, keyed_listings, keyed_walks) = preview(true);
    let (plain, plain_listings, plain_walks) = preview(false);
    let numbers = |folder: &crate::transfer::FolderConflictPreview| {
        [
            folder.new_files,
            folder.colliding_files,
            folder.newer_files,
            folder.equal_files,
            folder.target_newer_files,
            folder.conflict_files,
            folder.omitted,
        ]
    };
    assert_eq!(numbers(&keyed), [1, 6, 2, 4, 0, 0, 0], "{keyed:?}");
    assert_eq!(numbers(&keyed), numbers(&plain));
    assert_eq!(keyed.samples.len(), plain.samples.len());
    assert_eq!((keyed_walks, plain_walks), (1, 0));
    assert_eq!(
        keyed_listings, 3,
        "the selected folder, c and d: a, b, b/deep and d/e have equal keys"
    );
    assert_eq!(
        plain_listings, 7,
        "without keys every folder is listed: src, a, b, b/deep, c, d, d/e"
    );
}
