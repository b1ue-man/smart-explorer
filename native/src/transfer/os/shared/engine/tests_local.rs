//! Local → local through the engine: structure, conflicts, filter, links,
//! copies into themselves, moves, the copy entry points and the app trash.
use super::test_backend::{fwd, job, run, write};
use crate::transfer::{Endpoint, ResolvedRoot};
use crate::types::{Conflict, CopyMode, CopyOptions, FilterDef};
use std::fs;
use std::path::Path;
use std::time::Duration;

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("listing")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn vault(root: &Path) -> std::path::PathBuf {
    let vault = root.join("src/Vault");
    write(&vault.join("a.txt"), b"alpha");
    write(&vault.join("sub/b.txt"), b"beta");
    fs::create_dir_all(vault.join("empty")).expect("empty folder");
    vault
}

#[test]
fn transfer_engine_task_local_tree_copy_keeps_structure_and_empty_folders() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = vault(root.path());
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let finished = run(job(Endpoint::Local, Endpoint::Local, &dest, &[&source]));
    assert!(!finished.canceled);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert!(finished.errors.is_empty());
    assert!(finished.updates >= 1, "progress is reported");
    assert_eq!(
        (finished.progress.files_total, finished.progress.files_done),
        (2, 2)
    );
    assert_eq!(finished.progress.bytes_done, 9);
    assert!(finished.progress.done && !finished.progress.discovering);
    assert_eq!(fs::read(dest.join("Vault/a.txt")).expect("a"), b"alpha");
    assert_eq!(fs::read(dest.join("Vault/sub/b.txt")).expect("b"), b"beta");
    assert!(dest.join("Vault/empty").is_dir(), "empty folders stay");
    assert_eq!(names(&dest.join("Vault")), ["a.txt", "empty", "sub"]);
    assert_eq!(fs::read(source.join("a.txt")).expect("source"), b"alpha");
    assert_eq!(
        finished.roots,
        vec![ResolvedRoot {
            source: fwd(&source),
            rel: "Vault".to_string(),
        }]
    );
}

#[test]
fn transfer_engine_task_local_conflicts_follow_the_policy() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("src/note.txt");
    write(&source, b"new");
    for conflict in [Conflict::Rename, Conflict::Skip, Conflict::Overwrite] {
        let dest = root.path().join(format!("dest-{conflict:?}"));
        write(&dest.join("note.txt"), b"old");
        let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source]);
        transfer.conflict = conflict;
        let finished = run(transfer);
        assert!(
            finished.issues.is_empty(),
            "{conflict:?}: {:?}",
            finished.issues
        );
        match conflict {
            Conflict::Rename => {
                assert_eq!(fs::read(dest.join("note.txt")).expect("old"), b"old");
                assert_eq!(fs::read(dest.join("note (2).txt")).expect("copy"), b"new");
                assert_eq!(finished.progress.files_done, 1);
                assert_eq!(
                    finished.roots[0].rel, "note (2).txt",
                    "the resolved root is the numbered name"
                );
            }
            Conflict::Skip => {
                assert_eq!(fs::read(dest.join("note.txt")).expect("old"), b"old");
                assert_eq!(names(&dest), ["note.txt"]);
                assert_eq!(finished.progress.skipped, 1);
            }
            Conflict::Overwrite => {
                assert_eq!(fs::read(dest.join("note.txt")).expect("new"), b"new");
                assert_eq!(names(&dest), ["note.txt"]);
            }
        }
    }
}

#[test]
fn transfer_engine_task_local_filter_copies_matching_files_only() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = vault(root.path());
    write(&source.join("drop.md"), b"drop");
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let mut filter = FilterDef::new();
    filter.extensions = vec!["txt".to_string()];
    let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source]);
    transfer.filter = Some((filter, fwd(&root.path().join("src"))));
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 2);
    assert!(dest.join("Vault/a.txt").is_file());
    assert!(dest.join("Vault/sub/b.txt").is_file());
    assert!(!dest.join("Vault/drop.md").exists());
    assert!(
        !dest.join("Vault/empty").exists(),
        "filtered copies keep no empty folders"
    );
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_local_links_are_reported_not_followed() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = vault(root.path());
    let outside = root.path().join("outside");
    write(&outside.join("secret.txt"), b"secret");
    std::os::unix::fs::symlink(&outside, source.join("folder-link")).expect("dir link");
    std::os::unix::fs::symlink(outside.join("secret.txt"), source.join("file-link"))
        .expect("file link");
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let finished = run(job(Endpoint::Local, Endpoint::Local, &dest, &[&source]));
    assert_eq!(finished.progress.files_done, 2, "the other files still go");
    assert_eq!(finished.issues.len(), 2, "{:?}", finished.issues);
    assert!(finished.has_issue("Links"));
    assert!(!dest.join("Vault/folder-link").exists());
    assert!(!dest.join("Vault/file-link").exists());
    assert!(!dest.join("Vault/folder-link/secret.txt").exists());
}

#[test]
fn transfer_engine_task_local_target_inside_source_is_refused() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = vault(root.path());
    let below = source.join("sub");
    let finished = run(job(Endpoint::Local, Endpoint::Local, &below, &[&source]));
    assert!(finished.has_issue("Das Ziel liegt in einer der Quellen"));
    assert_eq!(finished.progress.files_done, 0);
    // Into its own parent: the copy would be the folder itself.
    let parent = root.path().join("src");
    let finished = run(job(Endpoint::Local, Endpoint::Local, &parent, &[&source]));
    assert!(finished.has_issue("Das Ziel liegt in einer der Quellen"));
    assert_eq!(names(&parent), ["Vault"]);
    #[cfg(unix)]
    {
        // A link to the source as target folder is refused before anything
        // is written through it.
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&source, &alias).expect("alias link");
        let finished = run(job(Endpoint::Local, Endpoint::Local, &alias, &[&source]));
        assert!(!finished.issues.is_empty());
        assert_eq!(finished.progress.files_done, 0);
    }
    assert_eq!(
        names(&source),
        ["a.txt", "empty", "sub"],
        "nothing was created"
    );
}

#[test]
fn transfer_engine_task_local_move_renames_whole_folders() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = vault(root.path());
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source]);
    transfer.mode = CopyMode::Move;
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert!(!source.exists(), "the folder moved as a whole");
    assert_eq!(
        fs::read(dest.join("Vault/sub/b.txt")).expect("moved"),
        b"beta"
    );
    assert!(dest.join("Vault/empty").is_dir());
    assert_eq!(finished.progress.files_done, 1, "one rename");
}

#[test]
fn transfer_engine_task_local_move_into_existing_folder_merges_and_prunes() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = vault(root.path());
    let dest = root.path().join("dest");
    write(&dest.join("Vault/kept.txt"), b"kept");
    let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source]);
    transfer.mode = CopyMode::Move;
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 2, "file by file");
    assert_eq!(fs::read(dest.join("Vault/a.txt")).expect("a"), b"alpha");
    assert_eq!(
        fs::read(dest.join("Vault/kept.txt")).expect("kept"),
        b"kept"
    );
    assert!(dest.join("Vault/empty").is_dir());
    assert!(!source.exists(), "emptied source folders are removed");
    assert!(
        root.path().join("src").is_dir(),
        "never beyond the selection"
    );
}

fn wait_copy(
    rx: &crossbeam_channel::Receiver<crate::copy::CopyMsg>,
) -> (crate::types::CopyProgress, Vec<(String, String)>) {
    loop {
        match rx
            .recv_timeout(Duration::from_secs(60))
            .expect("the copy ends in time")
        {
            crate::copy::CopyMsg::Progress(_) => {}
            crate::copy::CopyMsg::Done { progress, errors } => return (progress, errors),
        }
    }
}

#[test]
fn transfer_engine_task_copy_entry_points_run_on_the_engine() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = vault(root.path());
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let (tx, rx) = crossbeam_channel::unbounded();
    let options = CopyOptions {
        root: root.path().join("src"),
        dest: dest.clone(),
        preserve_structure: true,
        conflict: Conflict::Rename,
        mode: CopyMode::Copy,
    };
    let _handle = crate::copy::start_copy_from_paths(vec![fwd(&source)], options, tx);
    let (progress, errors) = wait_copy(&rx);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(progress.done && !progress.canceled);
    assert_eq!((progress.files_done, progress.files_total), (2, 2));
    assert!(dest.join("Vault/sub/b.txt").is_file());

    let pairs_dest = root.path().join("pairs");
    fs::create_dir(&pairs_dest).expect("pairs dest");
    let (tx, rx) = crossbeam_channel::unbounded();
    let pairs = vec![
        (fwd(&source.join("a.txt")), "Auswahl/a.txt".to_string()),
        (
            fwd(&source.join("sub/b.txt")),
            "Auswahl/tief/b.txt".to_string(),
        ),
    ];
    let _handle = crate::copy::start_copy_pairs(pairs, pairs_dest.clone(), Conflict::Rename, tx);
    let (progress, errors) = wait_copy(&rx);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(progress.files_done, 2);
    assert_eq!(
        fs::read(pairs_dest.join("Auswahl/tief/b.txt")).expect("pair"),
        b"beta"
    );

    let (tx, rx) = crossbeam_channel::unbounded();
    let invalid = vec![(fwd(&source.join("a.txt")), "../escape".to_string())];
    let _handle = crate::copy::start_copy_pairs(invalid, pairs_dest.clone(), Conflict::Rename, tx);
    let (progress, errors) = wait_copy(&rx);
    assert_eq!(progress.files_done, 0);
    assert!(
        !errors.is_empty(),
        "an invalid pair set is refused as a whole"
    );
    assert!(!root.path().join("escape").exists());
}

#[test]
fn transfer_engine_task_local_app_trash_is_left_out() {
    let root = tempfile::tempdir().expect("temp dir");
    crate::apptrash::set_volumes(vec![root.path().to_path_buf()]);
    let source = vault(root.path());
    write(
        &source.join(".SmartExplorer-Papierkorb/old.txt"),
        b"trashed",
    );
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let finished = run(job(Endpoint::Local, Endpoint::Local, &dest, &[&source]));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.omitted, 1);
    assert!(!dest.join("Vault/.SmartExplorer-Papierkorb").exists());
    assert_eq!(finished.progress.files_done, 2);
}
