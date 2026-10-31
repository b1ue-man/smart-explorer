//! Review fixes of the file rule "neuere ersetzen ältere" on the engine:
//! a replacement is bound to the target the decision saw (B1), a time one
//! side does not know decides nothing (W1), a name the target cannot list
//! keeps both and the rest of the folder runs (W3), and a stored job whose
//! folders merge under the rule is never read as "overwrite" by an older
//! version (B2).
use super::super::test_backend::{job, run, write, Fake};
use super::listings::listing_of;
use super::Found;
use crate::transfer::{count_folder_conflict, Endpoint, FolderConflict, FolderConflictPolicy};
use crate::types::Conflict;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, UNIX_EPOCH};

fn file(path: &Path, bytes: &[u8], seconds: u64) {
    write(path, bytes);
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open")
        .set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
        .expect("set time");
}

fn merging() -> FolderConflictPolicy {
    FolderConflictPolicy::new(FolderConflict::Merge)
}

/// Source `src/` with `up.txt` (newer) and the same name older at the target.
fn one_replacement(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let source = root.join("local/src");
    let dest = root.join("dest");
    file(&source.join("up.txt"), b"newer content", 6_000);
    file(&dest.join("src/up.txt"), b"old", 1_000);
    (source, dest)
}

fn upload(fake: Fake, source: &Path, dest: &Path) -> super::super::test_backend::Finished {
    let (_fake, handle) = fake.handle();
    let mut transfer = job(Endpoint::Local, Endpoint::Remote(handle), dest, &[source]);
    transfer.folders = merging();
    transfer.conflict = Conflict::Update;
    run(transfer)
}

#[test]
fn abgleich_task_an_unchanged_target_is_replaced_in_one_step() {
    let root = tempfile::tempdir().expect("temp dir");
    let (source, dest) = one_replacement(root.path());
    let mut fake = Fake::new("replace-control");
    fake.replaces = true;
    let finished = upload(fake, &source, &dest);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(fs::read(dest.join("src/up.txt")).unwrap(), b"newer content");
    assert!(!dest.join("src/up (2).txt").exists());
    assert_eq!(finished.progress.replaced, 1);
    assert_eq!(finished.progress.kept_both, 0);
}

#[test]
fn abgleich_task_a_target_saved_again_before_the_replacement_is_not_replaced() {
    let root = tempfile::tempdir().expect("temp dir");
    let (source, dest) = one_replacement(root.path());
    let saved = dest.join("src/up.txt");
    let mut fake = Fake::new("replace-race");
    fake.replaces = true;
    // The person saves the target again while the job transfers the source:
    // the stage is opened after the decision and before the replacement.
    *fake.faults.on_stage.lock().expect("hook") = Some(Box::new(move |_| {
        file(&saved, b"saved meanwhile", 9_000);
    }));
    let finished = upload(fake, &source, &dest);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(
        fs::read(dest.join("src/up.txt")).unwrap(),
        b"saved meanwhile",
        "the newer save of the person is never replaced"
    );
    assert_eq!(
        fs::read(dest.join("src/up (2).txt")).unwrap(),
        b"newer content",
        "the copy is kept beside it"
    );
    assert_eq!(finished.progress.replaced, 0);
    assert_eq!(finished.progress.kept_both, 1);
}

#[test]
fn abgleich_task_unknown_times_keep_both_even_at_the_same_size() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("local/src");
    let dest = root.path().join("dest");
    file(&source.join("same.txt"), b"same", 5_000);
    file(&dest.join("src/same.txt"), b"same", 5_000);
    let mut fake = Fake::new("unknown-times");
    fake.faults.unknown_times = true;
    let finished = upload(fake, &source, &dest);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(fs::read(dest.join("src/same.txt")).unwrap(), b"same");
    assert_eq!(
        fs::read(dest.join("src/same (2).txt")).unwrap(),
        b"same",
        "nothing proves them equal: the file is kept, not skipped"
    );
    assert_eq!(finished.progress.skipped_equal, 0);
    assert_eq!(finished.progress.kept_both, 1);
}

#[test]
fn abgleich_task_a_name_the_target_cannot_list_keeps_both_and_the_folder_runs() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("local/src");
    let dest = root.path().join("dest");
    file(&source.join("locked.txt"), b"from source", 6_000);
    file(&source.join("fresh.txt"), b"fresh", 6_000);
    file(&source.join("up.txt"), b"newer content", 6_000);
    file(&dest.join("src/locked.txt"), b"theirs", 1_000);
    file(&dest.join("src/up.txt"), b"old", 1_000);
    let mut fake = Fake::new("omitted-name");
    fake.replaces = true;
    fake.faults.omit_names = vec!["locked.txt".to_string()];
    let finished = upload(fake, &source, &dest);
    assert!(finished.errors.is_empty(), "{:?}", finished.errors);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(fs::read(dest.join("src/locked.txt")).unwrap(), b"theirs");
    assert_eq!(
        fs::read(dest.join("src/locked (2).txt")).unwrap(),
        b"from source"
    );
    assert_eq!(fs::read(dest.join("src/fresh.txt")).unwrap(), b"fresh");
    assert_eq!(
        fs::read(dest.join("src/up.txt")).unwrap(),
        b"newer content",
        "the other files of the folder are decided as usual"
    );
}

#[test]
fn abgleich_task_the_preview_counts_an_unlistable_name_as_a_conflict() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("local/src");
    let dest = root.path().join("dest");
    file(&source.join("locked.txt"), b"from source", 6_000);
    file(&source.join("fresh.txt"), b"fresh", 6_000);
    file(&source.join("up.txt"), b"newer content", 6_000);
    file(&dest.join("src/locked.txt"), b"theirs", 1_000);
    file(&dest.join("src/up.txt"), b"old", 1_000);
    let mut fake = Fake::new("omitted-preview");
    fake.faults.omit_names = vec!["locked.txt".to_string()];
    let (_fake, handle) = fake.handle();
    let mut transfer = job(Endpoint::Local, Endpoint::Remote(handle), &dest, &[&source]);
    transfer.folders = merging();
    assert!(!transfer.applies_update_rule());
    transfer.folders.merge_update = true;
    assert!(transfer.applies_update_rule());
    let stop = AtomicBool::new(false);
    let mut folders = crate::transfer::folder_conflicts(&transfer, &stop).expect("roots");
    let mut folder = folders.remove(0);
    count_folder_conflict(&transfer, &mut folder, &stop, &|_| {});
    assert!(folder.counted, "{folder:?}");
    assert_eq!(folder.new_files, 1, "fresh.txt");
    assert_eq!(folder.newer_files, 1, "up.txt");
    assert_eq!(
        folder.conflict_files, 1,
        "locked.txt is present but unknown"
    );
}

#[test]
fn abgleich_task_an_omitted_name_is_found_as_other_whatever_its_case() {
    let listing = listing_of(false, Vec::new(), &["Locked.TXT"]);
    assert!(matches!(listing.find("locked.txt"), Found::Other));
    assert!(matches!(listing.find("other.txt"), Found::Absent));
    let exact = listing_of(true, Vec::new(), &["Locked.TXT"]);
    assert!(matches!(exact.find("locked.txt"), Found::Absent));
    assert!(matches!(exact.find("Locked.TXT"), Found::Other));
}

#[test]
fn abgleich_task_a_merge_job_stores_keep_both_for_older_versions_and_reads_every_choice_back() {
    use crate::transfer::{file_rule, JobRecord};
    let record = |policy: u8, update: bool, choice: Option<u8>, merge_update: bool| -> JobRecord {
        let mut stored = serde_json::json!({
            "version": 1, "id": "x",
            "source": {"local": true, "namespace": "local", "state": "local"},
            "target": {"local": true, "namespace": "local", "state": "local"},
            "target_dir": "/t", "target_object": null, "vault_locations": [],
            "items": {"Roots": {"paths": ["/s"], "base": null}}, "layout": "Tree", "filter": null,
            "file_policy": policy, "file_update": update,
            "folders": {"default": "merge", "roots": {}},
            "moving": false, "guards": [], "complete": false, "mutation_started": false
        });
        if let Some(choice) = choice {
            stored["file_choice"] = serde_json::json!(choice);
        }
        if merge_update {
            stored["folders"]["merge_update"] = serde_json::json!(true);
        }
        serde_json::from_value(stored).expect("a record")
    };
    for conflict in [
        Conflict::Rename,
        Conflict::Overwrite,
        Conflict::Skip,
        Conflict::Update,
    ] {
        let (policy, update, choice) = file_rule(conflict, true);
        assert_eq!(
            (policy, update),
            (0, false),
            "a version that predates the rule reads \"keep both\" and never overwrites or skips"
        );
        let stored = record(policy, update, choice, true);
        let bound = stored
            .bind(Endpoint::Local, Endpoint::Local)
            .expect("binds");
        assert_eq!(
            bound.conflict, conflict,
            "the new version reads the choice back"
        );
        assert!(bound.folders.merge_update);
    }
    // Without merge_update the stored form is the one older versions know.
    assert_eq!(file_rule(Conflict::Overwrite, false), (1, false, None));
    assert_eq!(file_rule(Conflict::Skip, false), (2, false, None));
    assert_eq!(file_rule(Conflict::Update, false), (0, true, None));
    assert_eq!(file_rule(Conflict::Rename, false), (0, false, None));
    // A choice without merge_update, or an unknown one, is refused.
    assert!(record(0, false, Some(1), false)
        .bind(Endpoint::Local, Endpoint::Local)
        .is_err());
    assert!(record(0, false, Some(9), true)
        .bind(Endpoint::Local, Endpoint::Local)
        .is_err());
    assert!(record(1, false, Some(1), true)
        .bind(Endpoint::Local, Endpoint::Local)
        .is_err());
}
