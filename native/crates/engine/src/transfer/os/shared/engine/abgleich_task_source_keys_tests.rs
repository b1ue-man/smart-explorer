//! Folder keys with a tally on the source side (`KeyTally` of the keys-only
//! walk): with a remote source too, an equal folder is listed at neither side
//! and counts as found, equal and skipped at once, exactly as file by file; a
//! folder with an omission or a file of unknown time below it takes the
//! listing with its message, a filtered job lists as before, and the folder
//! decision's preview counts the same without listing the equal source
//! folders.
use super::super::test_backend::{fwd, job, run, Fake, Finished};
use super::keys_tests::{counts, file, files_under, merging, trees};
use crate::transfer::{count_folder_conflict, folder_conflicts, Endpoint};
use crate::types::{Conflict, FilterDef};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

struct Between {
    finished: Finished,
    source_lists: usize,
    source_walks: usize,
    target_walks: usize,
    tree: BTreeMap<String, Vec<u8>>,
}

/// Merges `source` into `dest` from one fake remote to another, both
/// answering the keyed walk or neither; `filter` as the job's filter.
fn between(source: &Path, dest: &Path, keyed: bool, filter: Option<FilterDef>) -> Between {
    let mut from = Fake::new("keys-from");
    from.faults.keyed = keyed;
    let (from, from_handle) = from.handle();
    let mut to = Fake::new("keys-to");
    to.replaces = true;
    to.faults.keyed = keyed;
    let (to, to_handle) = to.handle();
    let mut transfer = job(
        Endpoint::Remote(from_handle),
        Endpoint::Remote(to_handle),
        dest,
        &[source],
    );
    transfer.folders = merging();
    transfer.conflict = Conflict::Update;
    transfer.filter = filter.map(|filter| {
        let prefix = source.parent().expect("source parent");
        (filter, fwd(prefix))
    });
    let finished = run(transfer);
    Between {
        finished,
        source_lists: from.counters.lists.load(Ordering::SeqCst),
        source_walks: from.counters.key_walks.load(Ordering::SeqCst),
        target_walks: to.counters.key_walks.load(Ordering::SeqCst),
        tree: files_under(dest),
    }
}

/// Everything a surface counts, not only the file rule's numbers.
fn totals(finished: &Finished) -> [u64; 5] {
    let progress = &finished.progress;
    [
        progress.files_total,
        progress.files_done,
        progress.skipped,
        progress.omitted,
        progress.skipped_equal,
    ]
}

/// The issues of a run in `root` (its own temporary folder), each path
/// relative to that folder so two runs compare.
#[cfg(unix)]
fn issues(finished: &Finished, root: &str) -> Vec<(String, String)> {
    let mut issues: Vec<(String, String)> = finished
        .issues
        .iter()
        .map(|issue| {
            let path = issue.path.strip_prefix(root).unwrap_or(&issue.path);
            (path.to_string(), issue.message.clone())
        })
        .collect();
    issues.sort();
    issues
}

#[test]
fn abgleich_task_equal_folders_are_not_listed_at_the_source_either() {
    let with = tempfile::tempdir().expect("temp dir");
    let without = tempfile::tempdir().expect("temp dir");
    let (source, dest) = trees(with.path());
    let keyed = between(&source, &dest, true, None);
    let (source_plain, dest_plain) = trees(without.path());
    let plain = between(&source_plain, &dest_plain, false, None);

    assert!(
        keyed.finished.issues.is_empty(),
        "{:?}",
        keyed.finished.issues
    );
    assert_eq!(keyed.finished.progress.replaced, 2);
    assert_eq!(keyed.finished.progress.skipped_equal, 4);
    assert_eq!(counts(&keyed.finished), counts(&plain.finished));
    assert_eq!(totals(&keyed.finished), totals(&plain.finished));
    assert_eq!(keyed.tree, plain.tree, "the same files, the same bytes");
    assert_eq!((keyed.source_walks, keyed.target_walks), (1, 1));
    assert_eq!((plain.source_walks, plain.target_walks), (0, 0));
    assert_eq!(
        keyed.source_lists, 3,
        "the selected folder, c and d: a, b and d/e are equal and settled by their tally"
    );
    assert_eq!(
        plain.source_lists, 7,
        "without keys every folder is listed: src, a, b, b/deep, c, d, d/e"
    );
}

#[test]
fn abgleich_task_an_equal_tree_is_not_listed_at_the_source_at_all() {
    let with = tempfile::tempdir().expect("temp dir");
    let without = tempfile::tempdir().expect("temp dir");
    let equal = |root: &Path| {
        let (source, dest) = trees(root);
        fs::remove_dir_all(dest.join("src/c")).expect("clean");
        fs::remove_file(dest.join("src/d/one.txt")).expect("clean");
        fs::remove_dir_all(source.join("c")).expect("clean");
        fs::remove_file(source.join("d/one.txt")).expect("clean");
        (source, dest)
    };
    let (source, dest) = equal(with.path());
    let before = files_under(&dest);
    let keyed = between(&source, &dest, true, None);
    let (source_plain, dest_plain) = equal(without.path());
    let plain = between(&source_plain, &dest_plain, false, None);

    assert!(
        keyed.finished.issues.is_empty(),
        "{:?}",
        keyed.finished.issues
    );
    assert_eq!(
        keyed.source_lists, 0,
        "not even the selected folder is listed"
    );
    assert_eq!(keyed.tree, before, "nothing written, nothing duplicated");
    assert_eq!(totals(&keyed.finished), totals(&plain.finished));
    assert_eq!(counts(&keyed.finished), counts(&plain.finished));
    let progress = &keyed.finished.progress;
    assert_eq!((progress.files_total, progress.skipped_equal), (4, 4));
    assert_eq!((progress.bytes_total, progress.bytes_done), (15, 15));
    assert!(plain.source_lists >= 4, "{}", plain.source_lists);
}

#[test]
fn abgleich_task_a_filtered_job_still_lists_the_source() {
    let run_with = |keyed: bool| {
        let root = tempfile::tempdir().expect("temp dir");
        let (source, dest) = trees(root.path());
        // An equal folder holding a file the filter leaves out.
        for base in [&source, &dest.join("src")] {
            file(&base.join("a/skip.md"), b"markdown", 5_000);
        }
        let mut filter = FilterDef::new();
        filter.extensions = vec!["txt".to_string()];
        between(&source, &dest, keyed, Some(filter))
    };
    let (keyed, plain) = (run_with(true), run_with(false));
    assert_eq!(totals(&keyed.finished), totals(&plain.finished));
    assert_eq!(keyed.finished.progress.skipped_equal, 4, "no .md counted");
    assert_eq!(keyed.tree, plain.tree);
    assert_eq!(
        keyed.source_lists, plain.source_lists,
        "the filter walks it"
    );
}

/// A link inside an equal folder, a file of unknown time and a plain equal
/// folder: only the plain one is settled by its tally; the others are listed,
/// so the link is reported with its path and the unknown time keeps both.
#[cfg(unix)]
#[test]
fn abgleich_task_an_omission_or_unknown_time_takes_the_listing_with_its_message() {
    let run_with = |keyed: bool| {
        let root = tempfile::tempdir().expect("temp dir");
        let source = root.path().join("local/src");
        let dest = root.path().join("dest");
        for base in [&source, &dest.join("src")] {
            file(&base.join("a/one.txt"), b"one", 5_000);
            std::os::unix::fs::symlink("one.txt", base.join("a/link")).expect("link");
            file(&base.join("u/zero.txt"), b"zero", 0);
            file(&base.join("ok/f.txt"), b"fine", 5_000);
        }
        (between(&source, &dest, keyed, None), fwd(root.path()))
    };
    let ((keyed, keyed_root), (plain, plain_root)) = (run_with(true), run_with(false));
    assert!(
        keyed
            .finished
            .issues
            .iter()
            .any(|issue| issue.path.ends_with("a/link")),
        "the link is reported with its path: {:?}",
        keyed.finished.issues
    );
    assert_eq!(
        issues(&keyed.finished, &keyed_root),
        issues(&plain.finished, &plain_root)
    );
    assert_eq!(totals(&keyed.finished), totals(&plain.finished));
    assert_eq!(counts(&keyed.finished), counts(&plain.finished));
    assert_eq!(keyed.finished.progress.kept_both, 1, "zero.txt keeps both");
    assert_eq!(keyed.tree, plain.tree);
    assert_eq!(
        (keyed.source_lists, plain.source_lists),
        (3, 4),
        "src, a and u are listed; ok is settled by its tally"
    );
}

/// The folder decision's preview with a remote source: the same five counts,
/// and the equal source folders are not listed.
#[test]
fn abgleich_task_preview_does_not_list_equal_source_folders() {
    let preview = |keyed: bool| {
        let root = tempfile::tempdir().expect("temp dir");
        let (source, dest) = trees(root.path());
        let mut from = Fake::new("keys-preview-from");
        from.faults.keyed = keyed;
        let (from, from_handle) = from.handle();
        let mut to = Fake::new("keys-preview-to");
        to.faults.keyed = keyed;
        let (_to, to_handle) = to.handle();
        let mut transfer = job(
            Endpoint::Remote(from_handle),
            Endpoint::Remote(to_handle),
            &dest,
            &[&source],
        );
        transfer.conflict = Conflict::Update;
        let stop = AtomicBool::new(false);
        let mut folders = folder_conflicts(&transfer, &stop).expect("roots");
        let mut folder = folders.remove(0);
        count_folder_conflict(&transfer, &mut folder, &stop, &|_| {});
        assert!(folder.counted && folder.complete, "{folder:?}");
        (folder, from.counters.lists.load(Ordering::SeqCst))
    };
    let (keyed, keyed_lists) = preview(true);
    let (plain, plain_lists) = preview(false);
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
    assert_eq!(
        (keyed_lists, plain_lists),
        (3, 7),
        "the selected folder, c and d against every folder"
    );
}
