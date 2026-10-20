//! Folders and deletions in the comparison (stage `vault_compare`, vault
//! plan C6.4, R2, R3, R4): merging and keeping two folders of one name, the
//! deletions the rules hold back and their confirmation, restoring, and the
//! order of the apply phases. After every action the pair is done.
use std::sync::atomic::AtomicBool;

use super::compare_test_support::{item, stored, Fx};
use super::session_test_support::local_resolve;
use super::sync_job::{apply_vault_preview_action, prepare, preview_vault_job_in};
use crate::vault::core::compare_model::{CompareGroup, ItemAction, ItemKind};

/// A run finds nothing to do, nothing open, and both sides read alike.
fn assert_converged(fx: &Fx) {
    let run = fx.run();
    assert!(run.unavailable.is_none(), "{:?}", run.unavailable);
    assert!(run.outcome.errors.is_empty(), "{:?}", run.outcome.errors);
    assert!(
        run.outcome.conflicts.is_empty(),
        "{:?}",
        run.outcome.conflicts
    );
    assert!(
        run.outcome.deferred.is_empty(),
        "{:?}",
        run.outcome.deferred
    );
    let again = fx.run();
    let stats = &again.outcome.stats;
    assert_eq!((stats.a_to_b, stats.b_to_a, stats.deleted), (0, 0, 0));
    assert_eq!(fx.tree(&fx.root_a), fx.tree(&fx.root_b));
}

fn files_below(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_below(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Both sides create the folder `/Projekt`, each with its own file.
fn two_folders() -> Fx {
    let fx = Fx::new();
    fx.write(&fx.root_a, "/Projekt/a.txt", b"a");
    fx.write(&fx.root_b, "/Projekt/b.txt", b"b");
    fx
}

fn remove_backups(fx: &Fx) {
    for site in [&fx.site_a.id, &fx.site_b.id] {
        let _ = std::fs::remove_dir_all(super::compare_files::removed_root().join(site));
    }
}

#[test]
fn vault_task_compare_folder_merge_reencrypts_names_only() {
    let fx = two_folders();
    let view = fx.compare(&[CompareGroup::Conflict]);
    let folder = item(&view, "/Projekt");
    assert_eq!(folder.kind, ItemKind::Folder);
    assert_eq!(
        folder.actions,
        [ItemAction::MergeFolders, ItemAction::KeepFoldersBoth]
    );
    let before_a = stored(&fx.root_a);
    let before_b = stored(&fx.root_b);
    // The cipher file of b.txt: only on B, no node file, no `dirid.c9r`.
    let b_file = before_b
        .iter()
        .find(|(rel, _)| {
            !before_a.contains_key(*rel)
                && !rel.ends_with("/dir.c9r")
                && !rel.ends_with("/dirid.c9r")
        })
        .map(|(rel, bytes)| (rel.clone(), bytes.clone()))
        .expect("the file of B");
    // The user is asked first: a merge removes the second folder.
    assert!(fx.resolve(folder, "merge-folders", false).is_err());
    fx.resolve(folder, "merge-folders", true).expect("merged");
    assert_converged(&fx);
    let (a, b) = (fx.tree(&fx.root_a), fx.tree(&fx.root_b));
    assert_eq!(a["/Projekt/a.txt"], b"a");
    assert_eq!(a["/Projekt/b.txt"], b"b");
    assert_eq!(a, b);
    // The name was encrypted anew (another folder ID), the cipher bytes of
    // the file are the old ones.
    let after = stored(&fx.root_a);
    let moved = after
        .iter()
        .find(|(_, bytes)| **bytes == b_file.1)
        .map(|(rel, _)| rel.clone())
        .expect("the same cipher bytes");
    assert_ne!(moved, b_file.0);
    assert!(!after.contains_key(&b_file.0), "the old entry is gone");
    let test = "vault_task_compare_folder_merge_reencrypts_names_only";
    fx.keep(test, "a", &fx.root_a);
    fx.keep(test, "b", &fx.root_b);
    // A copy of what was removed stays in the removal folder.
    let mut kept = Vec::new();
    files_below(
        &super::compare_files::removed_root().join(&fx.site_b.id),
        &mut kept,
    );
    assert!(!kept.is_empty(), "a copy of the removed cipher files");
    remove_backups(&fx);
}

#[test]
fn vault_task_compare_folders_keep_both_renames_node_of_b() {
    let fx = two_folders();
    let view = fx.compare(&[CompareGroup::Conflict]);
    fx.resolve(item(&view, "/Projekt"), "keep-folders-both", false)
        .expect("both folders kept");
    assert_converged(&fx);
    let a = fx.tree(&fx.root_a);
    assert_eq!(a["/Projekt/a.txt"], b"a");
    let renamed: Vec<&String> = a
        .keys()
        .filter(|path| path.starts_with("/Projekt (Konflikt "))
        .collect();
    assert_eq!(renamed.len(), 1, "{:?}", a.keys());
    assert!(renamed[0].ends_with(" Tresor B)/b.txt"), "{}", renamed[0]);
    assert_eq!(a, fx.tree(&fx.root_b));
    let test = "vault_task_compare_folders_keep_both_renames_node_of_b";
    fx.keep(test, "a", &fx.root_a);
    fx.keep(test, "b", &fx.root_b);
    remove_backups(&fx);
}

/// `/Alt` deleted on A, changed on B: the rules hold the deletions back.
fn deleted_and_changed() -> Fx {
    let fx = Fx::bare();
    fx.write(&fx.root_a, "/Alt/x.txt", b"x");
    fx.write(&fx.root_a, "/Alt/y.txt", b"y");
    fx.write(&fx.root_a, "/bleibt.txt", b"bleibt");
    let fx = fx.settle();
    fx.remove_folder(&fx.root_a, "/Alt");
    fx.write(&fx.root_b, "/Alt/x.txt", b"x, auf B geaendert");
    fx
}

#[test]
fn vault_task_compare_deferred_confirm_and_restore() {
    // Confirm: A's deletion is taken over.
    let fx = deleted_and_changed();
    let view = fx.compare(&[]);
    let folder = item(&view, "/Alt");
    assert_eq!(folder.group, CompareGroup::Deferred, "{folder:?}");
    assert_eq!(item(&view, "/Alt/y.txt").group, CompareGroup::Deferred);
    let changed = item(&view, "/Alt/x.txt");
    assert_eq!(changed.group, CompareGroup::Conflict);
    assert!(!changed.a.exists && changed.b.exists);
    assert_eq!(
        folder.actions,
        [ItemAction::ConfirmDelete, ItemAction::Restore]
    );
    // The question comes first.
    let asked = fx
        .resolve(folder, "confirm-delete", false)
        .expect_err("asked");
    assert!(
        asked.to_string().contains("Trotzdem übernehmen?"),
        "{asked}"
    );
    fx.resolve(changed, "keep-a", false)
        .expect("deletion of x.txt");
    fx.resolve(item(&view, "/Alt/y.txt"), "confirm-delete", true)
        .expect("y.txt");
    fx.resolve(folder, "confirm-delete", true)
        .expect("the folder");
    assert!(!fx
        .tree(&fx.root_b)
        .keys()
        .any(|path| path.starts_with("/Alt")));
    assert_eq!(fx.tree(&fx.root_a), fx.tree(&fx.root_b));
    assert_converged(&fx);
    drop(fx);

    // Restore: what B still has comes back to A.
    let fx = deleted_and_changed();
    let view = fx.compare(&[]);
    fx.resolve(item(&view, "/Alt"), "restore", false)
        .expect("folder");
    fx.resolve(item(&view, "/Alt/y.txt"), "restore", false)
        .expect("y.txt");
    fx.resolve(item(&view, "/Alt/x.txt"), "keep-b", false)
        .expect("x.txt");
    let a = fx.tree(&fx.root_a);
    assert_eq!(a["/Alt/x.txt"], b"x, auf B geaendert");
    assert_eq!(a["/Alt/y.txt"], b"y");
    assert_eq!(a, fx.tree(&fx.root_b));
    assert_converged(&fx);
}

#[test]
fn vault_task_compare_restore_copies_back() {
    let fx = deleted_and_changed();
    let view = fx.compare(&[]);
    let before_b = stored(&fx.root_b);
    let files_a = stored(&fx.root_a).len();
    fx.resolve(item(&view, "/Alt"), "restore", false)
        .expect("restore");
    // The bytes B kept were copied to A unchanged; B did not change.
    assert_eq!(stored(&fx.root_b), before_b);
    let after_a = stored(&fx.root_a);
    assert!(after_a.len() > files_a);
    for (rel, bytes) in after_a
        .iter()
        .filter(|(rel, _)| before_b.contains_key(*rel))
    {
        if rel.ends_with("/dirid.c9r") || rel.ends_with("/dir.c9r") {
            assert_eq!(Some(bytes), before_b.get(rel), "{rel}");
        }
    }
    assert!(fx
        .compare(&[])
        .items
        .iter()
        .all(|entry| entry.path != "/Alt"));
}

#[test]
fn vault_task_compare_confirm_delete_bypasses_deferral_keeps_order() {
    let fx = deleted_and_changed();
    let view = fx.compare(&[]);
    let folder = item(&view, "/Alt");
    assert_eq!(folder.group, CompareGroup::Deferred);
    // The single apply of the vault sync refuses an action the rules hold
    // back; the confirmed deletion goes around that refusal …
    let cancel = AtomicBool::new(false);
    let vault_preview = preview_vault_job_in(&fx.store, &local_resolve, &fx.job, &cancel);
    let sides = prepare(&fx.store, &local_resolve, &fx.job)
        .unwrap_or_else(|_| panic!("both sides are reachable"));
    let (held, reason) = vault_preview
        .deferred
        .first()
        .expect("a deferred action")
        .clone();
    let refused = apply_vault_preview_action(
        sides.a.0.as_ref(),
        &sides.a.1,
        sides.b.0.as_ref(),
        &sides.b.1,
        &vault_preview,
        &[],
        &held,
        &fx.job,
        &cancel,
    )
    .expect_err("deferred actions are refused");
    assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(refused.to_string().contains(&reason));
    // … but the order of R4 still holds: a deletion below `d/` waits for the
    // node deletion of its folder while that is open.
    let node = vault_preview
        .deferred
        .iter()
        .map(|(action, _)| action.clone())
        .find(|action| {
            matches!(action, crate::bisync::Action::DeleteB(rel) if rel.ends_with("/dir.c9r"))
        })
        .expect("the node deletion");
    let content = vault_preview
        .deferred
        .iter()
        .map(|(action, _)| action.clone())
        .find(|action| {
            matches!(action, crate::bisync::Action::DeleteB(rel) if rel.ends_with("/dirid.c9r"))
        })
        .expect("the deletion of dirid.c9r");
    let waits = super::compare_engine::apply_confirmed_deletion(
        sides.a.0.as_ref(),
        &sides.a.1,
        sides.b.0.as_ref(),
        &sides.b.1,
        &vault_preview,
        std::slice::from_ref(&node),
        &content,
        &fx.job,
        &cancel,
    )
    .expect_err("the content waits for the node");
    assert_eq!(waits.kind(), std::io::ErrorKind::WouldBlock, "{waits}");
    // Through the comparison all of it works, in order: node first.
    fx.resolve(item(&view, "/Alt/x.txt"), "keep-a", false)
        .expect("x.txt");
    fx.resolve(item(&view, "/Alt/y.txt"), "confirm-delete", true)
        .expect("y.txt");
    fx.resolve(folder, "confirm-delete", true)
        .expect("the folder");
    assert_eq!(fx.tree(&fx.root_a), fx.tree(&fx.root_b));
}
