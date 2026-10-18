//! Vault sync jobs on real vaults created by se (stage `vault_sync`, vault
//! plan C5): seeding a copy, one-sided changes, conflicts that are never
//! resolved on their own, folders created on both sides, a folder deleted on
//! one side and changed on the other, key files changed on both sides and
//! different vaults.
use super::test_support::{vault_files, Fixture, PASSWORD};
use super::*;
use crate::vault::core::registry_types::SiteKind;
use crate::vault::core::sync_rules::{DEFERRED_FOLDER_CONFLICT, DEFERRED_NODE_DELETION};
use crate::vfs::LocalBackend;

fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

fn assert_clean(outcome: &VaultRunOutcome) {
    assert!(outcome.unavailable.is_none(), "{:?}", outcome.unavailable);
    assert!(
        outcome.outcome.errors.is_empty(),
        "{:?}",
        outcome.outcome.errors
    );
    assert!(
        outcome.outcome.conflicts.is_empty(),
        "{:?}",
        outcome.outcome.conflicts
    );
    assert!(outcome.planned_with_rules >= 1);
}

/// A seeded copy of a vault with three files, and its job.
fn seeded() -> (Fixture, SyncJob, String) {
    let f = Fixture::new();
    f.write(&f.root_a, "/Bericht.txt", b"Inhalt A");
    f.write(&f.root_a, "/Ordner/Tief/x.bin", &pattern(70_000));
    f.write(&f.root_a, "/leer.txt", b"");
    let (job, site_b) = f.replica_job();
    assert_clean(&f.run(&job));
    let root_b = site_b.location.clone();
    (f, job, root_b)
}

#[test]
fn vault_task_sync_seed_replica_then_seeding_false() {
    let f = Fixture::new();
    f.write(&f.root_a, "/Bericht.txt", b"Inhalt A");
    f.write(&f.root_a, "/Ordner/Tief/x.bin", &pattern(70_000));
    f.write(&f.root_a, "/leer.txt", b"");
    let (job, site_b) = f.replica_job();
    assert_eq!(site_b.kind, SiteKind::Replica);
    assert_eq!(site_b.replica_of.as_deref(), Some(f.site_a.id.as_str()));
    assert!(site_b.seeding, "a new copy is seeded");
    assert_eq!(site_b.vault_id, f.site_a.vault_id);
    let root_b = site_b.location.clone();
    assert!(vault_files(&root_b).is_empty(), "the copy starts empty");
    assert!(
        !root_b.starts_with(&f.path("vaults")),
        "copies never lie in the app data folder"
    );

    let first = f.run(&job);
    assert_clean(&first);
    assert_eq!(first.seeded.as_deref(), Some(site_b.id.as_str()));
    assert!(!f.record(&site_b.id).seeding, "seeding=false after the run");
    assert_eq!(first.outcome.stats.deleted, 0, "seeding never deletes");
    // The copy holds exactly the encrypted storage, and decrypts alike.
    assert_eq!(vault_files(&root_b), vault_files(&f.root_a));
    assert_eq!(f.tree(&root_b), f.tree(&f.root_a));
    assert_eq!(f.tree(&root_b)["/Bericht.txt"], b"Inhalt A");

    let second = f.run(&job);
    assert_clean(&second);
    let stats = &second.outcome.stats;
    assert_eq!((stats.a_to_b, stats.b_to_a, stats.deleted), (0, 0, 0));
    assert!(second.seeded.is_none());
    f.keep(
        "vault_task_sync_seed_replica_then_seeding_false",
        "a",
        &f.root_a,
        &[],
    );
    f.keep(
        "vault_task_sync_seed_replica_then_seeding_false",
        "b",
        &root_b,
        &[],
    );
}

#[test]
fn vault_task_sync_one_sided_changes_both_directions() {
    let (f, job, root_b) = seeded();
    // Changes on A: new, changed, deleted file, renamed folder.
    f.write(&f.root_a, "/neu.txt", b"von A");
    f.write(&f.root_a, "/Bericht.txt", b"Inhalt A, Fassung 2");
    f.view(&f.root_a).remove_file("/leer.txt").unwrap();
    f.view(&f.root_a).rename("/Ordner", "/Mappe").unwrap();
    let to_b = f.run(&job);
    assert_clean(&to_b);
    assert!(to_b.outcome.stats.a_to_b > 0 && to_b.outcome.stats.deleted > 0);
    assert_eq!(f.tree(&root_b), f.tree(&f.root_a));
    assert!(f.tree(&root_b).contains_key("/Mappe/Tief/x.bin"));

    // Changes on B flow back to A.
    f.write(&root_b, "/von-b.txt", b"von B");
    f.write(&root_b, "/neu.txt", b"von A, in B geaendert");
    f.view(&root_b).remove_file("/Mappe/Tief/x.bin").unwrap();
    let to_a = f.run(&job);
    assert_clean(&to_a);
    assert!(to_a.outcome.stats.b_to_a > 0);
    let tree = f.tree(&f.root_a);
    assert_eq!(tree, f.tree(&root_b));
    assert_eq!(tree["/von-b.txt"], b"von B");
    assert_eq!(tree["/neu.txt"], b"von A, in B geaendert");
    assert!(!tree.contains_key("/Mappe/Tief/x.bin"));
    assert_eq!(vault_files(&root_b), vault_files(&f.root_a));
    f.keep(
        "vault_task_sync_one_sided_changes_both_directions",
        "a",
        &f.root_a,
        &[],
    );
    f.keep(
        "vault_task_sync_one_sided_changes_both_directions",
        "b",
        &root_b,
        &[],
    );
}

#[test]
fn vault_task_sync_both_changed_untouched_conflict() {
    let (f, job, root_b) = seeded();
    f.write(&f.root_a, "/Bericht.txt", b"Fassung von A");
    f.write(&root_b, "/Bericht.txt", b"Fassung von B!");
    let (before_a, before_b) = (vault_files(&f.root_a), vault_files(&root_b));
    for _ in 0..2 {
        let outcome = f.run(&job);
        assert!(outcome.unavailable.is_none());
        assert_eq!(
            outcome.outcome.conflicts.len(),
            1,
            "{:?}",
            outcome.outcome.conflicts
        );
        assert!(outcome.outcome.conflicts[0].rel.ends_with(".c9r"));
        assert_eq!(
            outcome.outcome.stats.a_to_b + outcome.outcome.stats.b_to_a,
            0
        );
        // Never resolved on its own: both sides keep their bytes, no copy.
        assert_eq!(vault_files(&f.root_a), before_a);
        assert_eq!(vault_files(&root_b), before_b);
    }
    assert_eq!(f.tree(&f.root_a)["/Bericht.txt"], b"Fassung von A");
    assert_eq!(f.tree(&root_b)["/Bericht.txt"], b"Fassung von B!");
    assert_eq!(f.tree(&f.root_a).len(), 3, "no conflict copy");
    f.keep(
        "vault_task_sync_both_changed_untouched_conflict",
        "a",
        &f.root_a,
        &[],
    );
    f.keep(
        "vault_task_sync_both_changed_untouched_conflict",
        "b",
        &root_b,
        &[],
    );
}

#[test]
fn vault_task_sync_folder_created_both_sides() {
    let (f, job, root_b) = seeded();
    // Both sides create the folder „Projekt“ on their own: one node name,
    // two directory IDs, two content folders.
    f.write(&f.root_a, "/Projekt/a.txt", b"a");
    f.write(&root_b, "/Projekt/b.txt", b"b");
    let outcome = f.run(&job);
    assert!(outcome.unavailable.is_none());
    let conflicts: Vec<&str> = outcome
        .outcome
        .conflicts
        .iter()
        .map(|item| item.rel.as_str())
        .collect();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert!(
        conflicts[0].ends_with("/dir.c9r"),
        "the node: {conflicts:?}"
    );
    // Neither node is overwritten; no content is lost on either side.
    assert_eq!(f.tree(&f.root_a)["/Projekt/a.txt"], b"a");
    assert_eq!(f.tree(&root_b)["/Projekt/b.txt"], b"b");
    let (a, b) = (vault_files(&f.root_a), vault_files(&root_b));
    for (rel, bytes) in a.iter().filter(|(rel, _)| rel.as_str() != conflicts[0]) {
        assert_eq!(b.get(rel), Some(bytes), "{rel} missing on B");
    }
    for (rel, bytes) in b.iter().filter(|(rel, _)| rel.as_str() != conflicts[0]) {
        assert_eq!(a.get(rel), Some(bytes), "{rel} missing on A");
    }
    assert_ne!(a[conflicts[0]], b[conflicts[0]]);
    // The other side's content folder has no node there until the
    // comparison mode merges the folders.
    let allowed = ["OrphanContentDir"];
    f.keep(
        "vault_task_sync_folder_created_both_sides",
        "a",
        &f.root_a,
        &allowed,
    );
    f.keep(
        "vault_task_sync_folder_created_both_sides",
        "b",
        &root_b,
        &allowed,
    );
}

#[test]
fn vault_task_sync_delete_vs_change_deferred() {
    let f = Fixture::new();
    f.write(&f.root_a, "/Alt/x.txt", b"x");
    f.write(&f.root_a, "/Alt/y.txt", b"y");
    f.write(&f.root_a, "/bleibt.txt", b"bleibt");
    let (job, site_b) = f.replica_job();
    assert_clean(&f.run(&job));
    let root_b = site_b.location.clone();
    f.remove_folder(&f.root_a, "/Alt");
    f.write(&root_b, "/Alt/x.txt", b"x, auf B geaendert");
    let before_b = vault_files(&root_b);
    for _ in 0..2 {
        let outcome = f.run(&job);
        assert!(outcome.unavailable.is_none());
        let reasons: Vec<&str> = outcome
            .outcome
            .deferred
            .iter()
            .map(|(_, reason)| reason.as_str())
            .collect();
        assert!(reasons.contains(&DEFERRED_FOLDER_CONFLICT), "{reasons:?}");
        assert!(reasons.contains(&DEFERRED_NODE_DELETION), "{reasons:?}");
        assert_eq!(outcome.outcome.conflicts.len(), 1, "the changed file");
        assert_eq!(outcome.outcome.stats.deleted, 0, "no deletion on B");
        assert_eq!(vault_files(&root_b), before_b, "B untouched");
    }
    let tree_b = f.tree(&root_b);
    assert_eq!(tree_b["/Alt/x.txt"], b"x, auf B geaendert");
    assert_eq!(tree_b["/Alt/y.txt"], b"y");
    assert!(!f.tree(&f.root_a).contains_key("/Alt/x.txt"));
    f.keep(
        "vault_task_sync_delete_vs_change_deferred",
        "a",
        &f.root_a,
        &[],
    );
    f.keep(
        "vault_task_sync_delete_vs_change_deferred",
        "b",
        &root_b,
        &[],
    );
}

#[test]
fn vault_task_sync_masterkey_both_changed_conflict() {
    let (f, job, root_b) = seeded();
    let local = LocalBackend::new("/");
    let jti = Some(f.site_a.vault_id.as_str());
    let changed_a = super::super::password::change_password(
        &local,
        &f.root_a,
        jti,
        PASSWORD,
        "Neues-Passwort-A",
        None,
    );
    let changed_b = super::super::password::change_password(
        &local,
        &root_b,
        jti,
        PASSWORD,
        "Neues-Passwort-B",
        None,
    );
    assert!(changed_a.is_ok() && changed_b.is_ok());
    let before = (vault_files(&f.root_a), vault_files(&root_b));
    let outcome = f.run(&job);
    assert!(outcome.unavailable.is_none());
    let conflicts: Vec<&str> = outcome
        .outcome
        .conflicts
        .iter()
        .map(|item| item.rel.as_str())
        .collect();
    assert_eq!(conflicts, [crate::cryptomator::MASTERKEY_FILE]);
    assert_eq!((vault_files(&f.root_a), vault_files(&root_b)), before);
    // Each side keeps its own key file and password.
    let unlock = |root: &str, password: &str| {
        super::super::unlock::unlock_storage(&local, root, password, None).is_ok()
    };
    assert!(unlock(&f.root_a, "Neues-Passwort-A") && !unlock(&f.root_a, "Neues-Passwort-B"));
    assert!(unlock(&root_b, "Neues-Passwort-B") && !unlock(&root_b, "Neues-Passwort-A"));
}

#[test]
fn vault_task_sync_jti_mismatch_aborts() {
    let f = Fixture::new();
    f.write(&f.root_a, "/a.txt", b"a");
    let other_root = f.path("fremd");
    let local = LocalBackend::new("/");
    let other = super::super::manage::create_vault(&local, &other_root, PASSWORD, 220, None)
        .expect("second vault");
    assert_ne!(other.jti, f.site_a.vault_id);
    let id = crate::vault::core::registry_types::new_site_id().unwrap();
    let site = SiteRecord::new(&id, "Fremd", &other_root, &other.jti, 3, f.store.defaults());
    f.store.add(&site).unwrap();
    let mut job = SyncJob::new_vault("Falsch".into(), &f.site_a.id, &id);
    job.id = "vaulttaskjtimismatch".into();
    let before = (vault_files(&f.root_a), vault_files(&other_root));
    let outcome = f.run(&job);
    let reason = outcome.unavailable.expect("run aborted");
    assert!(reason.contains("verschiedene Tresore"), "{reason}");
    assert!(outcome.outcome.stats.errors > 0);
    assert_eq!(outcome.planned_with_rules, 0, "nothing planned");
    assert_eq!((vault_files(&f.root_a), vault_files(&other_root)), before);
    assert!(!std::path::Path::new(&format!(
        "{}/{}",
        f.root_a,
        crate::bisync::REPLICA_MARKER_NAME
    ))
    .exists());
    // A site entered for another vault than stored at its location is
    // refused as well.
    let mut wrong = site.clone();
    wrong.vault_id = f.site_a.vault_id.clone();
    f.store.save(&wrong).unwrap();
    let outcome = f.run(&job);
    assert!(outcome.unavailable.is_some());
    assert_eq!((vault_files(&f.root_a), vault_files(&other_root)), before);
}
