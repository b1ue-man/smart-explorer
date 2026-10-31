//! The file rule "neuere ersetzen ältere" on the engine: every combination of
//! sides decides per file, the folder decision "Zusammenführen" brings it to
//! the files below it, and the protections of the policies underneath stay.
use super::super::test_backend::{fwd, job, run, write, Fake};
use crate::transfer::{Endpoint, FolderConflict, FolderConflictPolicy};
use crate::types::Conflict;
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

fn stamp(path: &Path, seconds: u64) {
    let file = fs::OpenOptions::new().write(true).open(path).expect("open");
    file.set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
        .expect("set time");
}

fn file(path: &Path, bytes: &[u8], seconds: u64) {
    write(path, bytes);
    stamp(path, seconds);
}

/// Source `src/` and destination `dest/src/` with one file per case.
fn tree(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let source = root.join("local/src");
    let dest = root.join("dest");
    file(&source.join("new.txt"), b"new", 5_000);
    file(&source.join("sub/deep.txt"), b"deep", 5_000);
    file(&source.join("up.txt"), b"newer content", 6_000);
    file(&source.join("same.txt"), b"same", 5_000);
    file(&source.join("keep.txt"), b"from source", 1_000);
    file(&source.join("clash.txt"), b"abc", 2_000);
    file(&dest.join("src/up.txt"), b"old", 1_000);
    file(&dest.join("src/same.txt"), b"same", 5_000);
    file(&dest.join("src/keep.txt"), b"destination is newer", 9_000);
    file(&dest.join("src/clash.txt"), b"abcde", 2_000);
    (source, dest)
}

fn merging() -> FolderConflictPolicy {
    FolderConflictPolicy::new(FolderConflict::Merge)
}

#[test]
fn abgleich_task_local_update_decides_each_case() {
    let root = tempfile::tempdir().expect("temp dir");
    let (source, dest) = tree(root.path());
    let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source]);
    transfer.folders = merging();
    transfer.conflict = Conflict::Update;
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    let target = dest.join("src");
    assert_eq!(fs::read(target.join("new.txt")).unwrap(), b"new");
    assert_eq!(fs::read(target.join("sub/deep.txt")).unwrap(), b"deep");
    assert_eq!(fs::read(target.join("up.txt")).unwrap(), b"newer content");
    assert_eq!(fs::read(target.join("same.txt")).unwrap(), b"same");
    assert_eq!(
        fs::read(target.join("keep.txt")).unwrap(),
        b"destination is newer"
    );
    assert_eq!(fs::read(target.join("clash.txt")).unwrap(), b"abcde");
    assert_eq!(fs::read(target.join("clash (2).txt")).unwrap(), b"abc");
    let progress = &finished.progress;
    assert_eq!(
        (
            progress.replaced,
            progress.skipped_equal,
            progress.kept_newer,
            progress.kept_both
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(progress.skipped, 2, "equal and newer targets are skipped");
    assert_eq!(
        progress.bytes_done, progress.bytes_total,
        "skipped files count as handled"
    );
    assert_eq!(fs::read(source.join("up.txt")).unwrap(), b"newer content");
}

#[test]
fn abgleich_task_merge_decision_brings_the_rule_to_the_files_below_only() {
    let root = tempfile::tempdir().expect("temp dir");
    let (source, dest) = tree(root.path());
    let loose = root.path().join("local/loose.txt");
    file(&loose, b"newer loose", 6_000);
    file(&dest.join("loose.txt"), b"old loose", 1_000);
    let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source, &loose]);
    transfer
        .folders
        .roots
        .insert(fwd(&source), FolderConflict::Merge);
    transfer.folders.merge_update = true;
    assert_eq!(transfer.conflict, Conflict::Rename);
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    let target = dest.join("src");
    assert_eq!(fs::read(target.join("up.txt")).unwrap(), b"newer content");
    assert_eq!(
        fs::read(target.join("keep.txt")).unwrap(),
        b"destination is newer"
    );
    assert_eq!(
        fs::read(dest.join("loose.txt")).unwrap(),
        b"old loose",
        "a file selected on its own keeps its policy"
    );
    assert_eq!(
        fs::read(dest.join("loose (2).txt")).unwrap(),
        b"newer loose",
        "and is kept beside the old one"
    );
    assert_eq!(finished.progress.replaced, 1);
}

#[test]
fn abgleich_task_without_the_flag_a_merged_folder_keeps_both_as_before() {
    let root = tempfile::tempdir().expect("temp dir");
    let (source, dest) = tree(root.path());
    let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source]);
    transfer
        .folders
        .roots
        .insert(fwd(&source), FolderConflict::Merge);
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    let target = dest.join("src");
    assert_eq!(fs::read(target.join("up.txt")).unwrap(), b"old");
    assert_eq!(
        fs::read(target.join("up (2).txt")).unwrap(),
        b"newer content"
    );
    assert_eq!(
        finished.progress.replaced + finished.progress.skipped_equal,
        0
    );
}

#[test]
fn abgleich_task_upload_skips_equal_and_newer_and_keeps_both_where_it_cannot_replace() {
    let root = tempfile::tempdir().expect("temp dir");
    let (source, dest) = tree(root.path());
    let (_fake, handle) = Fake::new("update-upload").handle();
    let mut transfer = job(Endpoint::Local, Endpoint::Remote(handle), &dest, &[&source]);
    transfer.folders = merging();
    transfer.conflict = Conflict::Update;
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    let target = dest.join("src");
    assert_eq!(fs::read(target.join("new.txt")).unwrap(), b"new");
    assert_eq!(fs::read(target.join("sub/deep.txt")).unwrap(), b"deep");
    assert_eq!(fs::read(target.join("same.txt")).unwrap(), b"same");
    assert_eq!(
        fs::read(target.join("keep.txt")).unwrap(),
        b"destination is newer"
    );
    assert_eq!(fs::read(target.join("clash.txt")).unwrap(), b"abcde");
    assert_eq!(fs::read(target.join("clash (2).txt")).unwrap(), b"abc");
    // The fixture cannot replace in one step: the newer file is kept beside
    // the old one, nothing is lost, and it is counted as kept, not replaced.
    assert_eq!(fs::read(target.join("up.txt")).unwrap(), b"old");
    assert_eq!(
        fs::read(target.join("up (2).txt")).unwrap(),
        b"newer content"
    );
    let progress = &finished.progress;
    assert_eq!(
        (
            progress.replaced,
            progress.skipped_equal,
            progress.kept_newer,
            progress.kept_both
        ),
        (0, 1, 1, 2)
    );
}

#[test]
fn abgleich_task_download_replaces_an_older_local_file() {
    let root = tempfile::tempdir().expect("temp dir");
    let (source, dest) = tree(root.path());
    let (_fake, handle) = Fake::new("update-download").handle();
    let mut transfer = job(Endpoint::Remote(handle), Endpoint::Local, &dest, &[&source]);
    transfer.folders = merging();
    transfer.conflict = Conflict::Update;
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    let target = dest.join("src");
    assert_eq!(fs::read(target.join("up.txt")).unwrap(), b"newer content");
    assert_eq!(fs::read(target.join("same.txt")).unwrap(), b"same");
    assert_eq!(
        fs::read(target.join("keep.txt")).unwrap(),
        b"destination is newer"
    );
    assert_eq!(fs::read(target.join("clash (2).txt")).unwrap(), b"abc");
    assert_eq!(finished.progress.replaced, 1);
    assert_eq!(finished.progress.skipped_equal, 1);
}

#[test]
fn abgleich_task_remote_to_remote_decides_per_file_too() {
    let root = tempfile::tempdir().expect("temp dir");
    let (source, dest) = tree(root.path());
    let (_one, from) = Fake::new("update-from").handle();
    let (_two, to) = Fake::new("update-to").handle();
    let mut transfer = job(
        Endpoint::Remote(from),
        Endpoint::Remote(to),
        &dest,
        &[&source],
    );
    transfer.folders = merging();
    transfer.conflict = Conflict::Update;
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    let target = dest.join("src");
    assert_eq!(fs::read(target.join("new.txt")).unwrap(), b"new");
    assert_eq!(fs::read(target.join("same.txt")).unwrap(), b"same");
    assert_eq!(finished.progress.skipped_equal, 1);
    assert_eq!(finished.progress.kept_newer, 1);
}

#[test]
fn abgleich_task_a_second_run_over_a_finished_merge_transfers_nothing() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("local/src");
    let dest = root.path().join("dest");
    file(&source.join("a.txt"), b"alpha", 5_000);
    file(&source.join("sub/b.txt"), b"beta", 5_000);
    file(&dest.join("src/b.txt"), b"old", 1_000);
    let make = || {
        let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source]);
        transfer.folders = merging();
        transfer.conflict = Conflict::Update;
        transfer
    };
    let first = run(make());
    assert!(first.issues.is_empty(), "{:?}", first.issues);
    assert_eq!(fs::read(dest.join("src/a.txt")).unwrap(), b"alpha");
    assert_eq!(fs::read(dest.join("src/sub/b.txt")).unwrap(), b"beta");
    let again = run(make());
    assert!(again.issues.is_empty(), "{:?}", again.issues);
    assert_eq!(
        again.progress.files_done, 0,
        "nothing newer, nothing copied again"
    );
    assert_eq!(
        again.progress.skipped, 2,
        "both are equal or newer at the destination"
    );
    assert_eq!(again.progress.kept_both, 0, "no duplicate on a repeat");
}

#[test]
fn abgleich_task_job_record_keeps_the_rule_and_old_records_read_as_before() {
    use crate::transfer::JobRecord;
    let stored = serde_json::json!({
        "version": 1, "id": "x", "source": {"local": true, "namespace": "local", "state": "local"},
        "target": {"local": true, "namespace": "local", "state": "local"},
        "target_dir": "/t", "target_object": null, "vault_locations": [],
        "items": {"Roots": {"paths": ["/s"], "base": null}}, "layout": "Tree", "filter": null,
        "file_policy": 0, "folders": {"default": "keep-both", "roots": {}},
        "moving": false, "guards": [], "complete": false, "mutation_started": false
    });
    let old: JobRecord = serde_json::from_value(stored.clone()).expect("an old record");
    assert!(!old.file_update, "a record without the field is false");
    assert!(!old.folders.merge_update);
    let mut with_rule = stored;
    with_rule["file_update"] = serde_json::json!(true);
    with_rule["folders"]["merge_update"] = serde_json::json!(true);
    let record: JobRecord = serde_json::from_value(with_rule).expect("a new record");
    let job = record
        .bind(Endpoint::Local, Endpoint::Local)
        .expect("binds");
    assert_eq!(job.conflict, Conflict::Update);
    assert!(job.folders.merge_update);
    let mut unknown = record.clone();
    unknown.file_policy = 1;
    assert!(
        unknown.bind(Endpoint::Local, Endpoint::Local).is_err(),
        "overwrite and update never combine"
    );
    let plain = serde_json::to_value(&old).expect("serialises");
    assert!(
        plain["folders"].get("merge_update").is_none(),
        "records without the rule stay as they were"
    );
}

fn seconds(path: &Path) -> u64 {
    fs::metadata(path)
        .expect("metadata")
        .modified()
        .expect("modified")
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs()
}

#[test]
fn abgleich_task_a_local_copy_carries_the_time_of_its_source() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("local/src");
    let dest = root.path().join("dest");
    file(&source.join("a.txt"), b"alpha", 1_000_000);
    file(&source.join("sub/b.txt"), b"beta", 2_000_000);
    fs::create_dir_all(&dest).expect("dest");
    let finished = run(job(Endpoint::Local, Endpoint::Local, &dest, &[&source]));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(seconds(&dest.join("src/a.txt")), 1_000_000);
    assert_eq!(seconds(&dest.join("src/sub/b.txt")), 2_000_000);
}

#[test]
fn abgleich_task_a_download_carries_the_time_of_its_source() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("remote/src");
    let dest = root.path().join("dest");
    file(&source.join("a.txt"), b"alpha", 1_000_000);
    fs::create_dir_all(&dest).expect("dest");
    let (_fake, handle) = Fake::new("time-download").handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(seconds(&dest.join("src/a.txt")), 1_000_000);
}

#[test]
fn abgleich_task_an_upload_gives_its_stage_the_time_of_its_source() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("local/src");
    let dest = root.path().join("dest");
    file(&source.join("a.txt"), b"alpha", 1_000_000);
    fs::create_dir_all(&dest).expect("dest");
    let mut fake = Fake::new("time-upload");
    fake.carries_time = true;
    let (_fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &dest,
        &[&source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(seconds(&dest.join("src/a.txt")), 1_000_000);
}

#[test]
fn abgleich_task_a_merge_back_after_a_copy_replaces_nothing() {
    let root = tempfile::tempdir().expect("temp dir");
    let a = root.path().join("a/tree");
    let b = root.path().join("b");
    file(&a.join("one.txt"), b"one", 1_000_000);
    file(&a.join("sub/two.txt"), b"two", 1_500_000);
    fs::create_dir_all(&b).expect("b");
    let mut forth = job(Endpoint::Local, Endpoint::Local, &b, &[&a]);
    forth.folders = merging();
    forth.conflict = Conflict::Update;
    assert!(run(forth).issues.is_empty());
    // B back into A's parent: the copies are the same files, not newer ones.
    let mut back = job(
        Endpoint::Local,
        Endpoint::Local,
        &root.path().join("a"),
        &[&b.join("tree")],
    );
    back.folders = merging();
    back.conflict = Conflict::Update;
    let finished = run(back);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.replaced, 0);
    assert_eq!(finished.progress.files_done, 0);
    assert_eq!(finished.progress.skipped_equal, 2);
}
