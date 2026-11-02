//! The comparison mode of vault sync jobs (stage `vault_compare`, vault plan
//! C6): which entries a comparison lists in which group, with which actions,
//! the numbers of the entries, the key side, the state the comparison keeps
//! and its end.
use sha2::{Digest, Sha256};

use super::compare::{compare_job_in, item_id};
use super::compare_cache::{cache, Payload, SideIo};
use super::compare_items::group_of;
use super::compare_test_support::{item, pattern, stored, tick, Fx};
use super::open::{publish_session, unlocked_session, withdraw_session};
use super::session_test_support::local_resolve;
use super::sync_job::preview_vault_job_in;
use crate::bisync::Action;
use crate::vault::core::compare_model::{CompareGroup, ItemAction, ItemKind};
use crate::vault::core::locked_site;
use crate::vault::core::VaultSession;
use crate::vfs::{Backend, LocalBackend};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Changes on both sides of the settled pair: one entry of each group that
/// a file can be in.
fn changed() -> Fx {
    let fx = Fx::new();
    fx.write(&fx.root_a, "/neu-a.txt", b"neu auf A");
    fx.write(&fx.root_a, "/Bericht.txt", b"Inhalt A, Fassung 2");
    fx.write(&fx.root_b, "/neu-b.txt", b"neu auf B");
    fx.write(&fx.root_b, "/leer.txt", b"jetzt nicht mehr leer");
    fx.view(&fx.root_b)
        .remove_file("/Ordner/Tief/x.bin")
        .expect("deleted on B");
    fx.write(&fx.root_a, "/konflikt.txt", b"Fassung von A");
    fx.write(&fx.root_b, "/konflikt.txt", b"Fassung von B!");
    fx
}

#[test]
fn vault_task_compare_groups_match_preview() {
    let fx = changed();
    let view = fx.compare(&[]);
    let group = |path: &str| item(&view, path).group;
    assert_eq!(group("/neu-a.txt"), CompareGroup::NewA);
    assert_eq!(group("/Bericht.txt"), CompareGroup::ChangedA);
    assert_eq!(group("/neu-b.txt"), CompareGroup::NewB);
    assert_eq!(group("/leer.txt"), CompareGroup::ChangedB);
    assert_eq!(group("/Ordner/Tief/x.bin"), CompareGroup::DeletedB);
    assert_eq!(group("/konflikt.txt"), CompareGroup::Conflict);
    // Every action and every conflict of the engine's preview is an entry.
    let preview = preview_vault_job_in(&fx.store, &local_resolve, &fx.job, &AtomicBool::new(false));
    let planned = preview.preview.actions.len() + preview.preview.conflicts.len();
    assert_eq!(view.items.len(), planned, "{:?}", view.items);
    assert_eq!(view.counts["conflict"], preview.preview.conflicts.len());
    let total: usize = view.counts.values().sum();
    assert_eq!(total, view.items.len());
    // The actions follow the group (C6.2).
    let actions = |path: &str| item(&view, path).actions.clone();
    assert_eq!(actions("/neu-a.txt"), [ItemAction::Transfer]);
    assert_eq!(
        actions("/konflikt.txt"),
        [
            ItemAction::KeepA,
            ItemAction::KeepB,
            ItemAction::KeepBoth,
            ItemAction::Merge
        ]
    );
    assert_eq!(
        actions("/Ordner/Tief/x.bin"),
        [ItemAction::ConfirmDelete, ItemAction::Restore]
    );
    assert!(view.items.iter().all(|entry| entry.kind != ItemKind::Node));
}

#[test]
fn vault_task_compare_groups_from_actions() {
    // Every kind of action lands in its group (U26-4), with the target side
    // deciding between new and changed.
    let dir = tempfile::tempdir().expect("temporary folder");
    let root = |name: &str| format!("{}/{name}", super::compare_test_support::fwd(dir.path()));
    let (root_a, root_b) = (root("a"), root("b"));
    for root in [&root_a, &root_b] {
        std::fs::create_dir_all(root).expect("side folder");
    }
    std::fs::write(format!("{root_a}/auf-beiden"), b"x").expect("file");
    std::fs::write(format!("{root_b}/auf-beiden"), b"y").expect("file");
    let side = |root: &String, label: &str| SideIo {
        backend: Arc::new(LocalBackend::new("/")),
        root: root.clone(),
        site: label.to_string(),
        label: label.to_string(),
    };
    let sides = [side(&root_a, "A"), side(&root_b, "B")];
    let group = |action: Action| group_of(&action, &[], &sides);
    let rel = |name: &str| name.to_string();
    assert_eq!(
        group(Action::CopyAtoB(rel("nur-auf-a"))),
        CompareGroup::NewA
    );
    assert_eq!(
        group(Action::CopyAtoB(rel("auf-beiden"))),
        CompareGroup::ChangedA
    );
    assert_eq!(
        group(Action::CopyBtoA(rel("nur-auf-b"))),
        CompareGroup::NewB
    );
    assert_eq!(
        group(Action::CopyBtoA(rel("auf-beiden"))),
        CompareGroup::ChangedB
    );
    assert_eq!(
        group(Action::FinalizeMoveAtoB(rel("x"))),
        CompareGroup::ChangedA
    );
    assert_eq!(
        group(Action::FinalizeMoveBtoA(rel("x"))),
        CompareGroup::ChangedB
    );
    // The deletion that B would follow is a deletion on A.
    assert_eq!(group(Action::DeleteB(rel("x"))), CompareGroup::DeletedA);
    assert_eq!(group(Action::DeleteA(rel("x"))), CompareGroup::DeletedB);
    // An action the rules hold back is deferred, whatever it would do.
    let held = Action::CopyAtoB(rel("auf-beiden"));
    assert_eq!(
        group_of(&held, &[(held.clone(), "Grund".into())], &sides),
        CompareGroup::Deferred
    );
}

#[test]
fn vault_task_compare_technical_files_fold_into_item() {
    let fx = Fx::new();
    fx.write(&fx.root_a, "/Neu/innen.txt", b"drinnen");
    let view = fx.compare(&[]);
    // One entry for the folder (its node file and its `dirid.c9r`), one for
    // the file inside; nothing without a name.
    assert_eq!(view.items.len(), 2, "{:?}", view.items);
    assert!(view.items.iter().all(|entry| !entry.path.is_empty()));
    let folder = item(&view, "/Neu");
    assert_eq!(folder.kind, ItemKind::Folder);
    assert_eq!(folder.group, CompareGroup::NewA);
    assert_eq!(item(&view, "/Neu/innen.txt").kind, ItemKind::File);
    let state = cache().get(&fx.job.id).expect("kept comparison");
    let items = state.items();
    let held = items
        .iter()
        .find(|held| held.item.path == "/Neu")
        .expect("folder entry");
    let Payload::Files { rels, .. } = &held.payload else {
        panic!("not a file entry");
    };
    assert!(
        rels.iter().any(|rel| rel.ends_with("/dirid.c9r")),
        "{rels:?}"
    );
    assert!(rels.iter().any(|rel| rel.ends_with("/dir.c9r")), "{rels:?}");
}

#[test]
fn vault_task_compare_item_ids_stable() {
    let fx = changed();
    let first = fx.compare(&[]);
    let second = fx.compare(&[]);
    let ids = |view: &crate::vault::core::compare_model::CompareView| {
        view.items
            .iter()
            .map(|entry| (entry.path.clone(), entry.id.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&first), ids(&second));
    let mut seen = std::collections::HashSet::new();
    for entry in &first.items {
        assert_eq!(entry.id.len(), 16, "{}", entry.id);
        assert!(entry
            .id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert!(seen.insert(entry.id.clone()), "numbers are unique");
    }
    // The number is the first 16 hex digits of the documented hash.
    let digest = Sha256::digest(b"se-vault-item\0job\0file\0/a.txt");
    let expected: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(item_id("job", "file", "/a.txt"), expected);
    // Sorted by group, then path.
    let keys: Vec<(CompareGroup, String)> = first
        .items
        .iter()
        .map(|entry| (entry.group, entry.path.clone()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}

#[test]
fn vault_task_compare_stale_after_new_compare() {
    let fx = changed();
    let first = fx.compare(&[]);
    let second = fx.compare(&[]);
    assert!(second.revision > first.revision);
    let stale = super::compare::compare_snapshot(&fx.job.id, first.revision)
        .expect_err("an older comparison is gone");
    assert!(super::compare_cache::is_stale(&stale));
    let listing =
        super::compare::compare_snapshot(&fx.job.id, second.revision).expect("the newer one");
    assert_eq!(listing.len(), second.items.len());
    // An action on a comparison nobody made is stale as well.
    let unknown = fx
        .resolve_request(Some("0123456789abcdef"), "transfer", false, false)
        .expect_err("no such entry");
    assert!(super::compare_cache::is_stale(&unknown), "{unknown}");
    let nothing = super::compare_actions::resolve_item_in(
        &super::compare_actions::ActionEnv {
            store: &fx.store,
            resolve: &local_resolve,
        },
        super::compare_actions::ResolveRequest {
            job: "gibt-es-nicht",
            item: Some("0123456789abcdef"),
            action: "transfer",
            all: false,
            confirmed: false,
        },
        &AtomicBool::new(false),
    )
    .expect_err("no comparison of that job");
    assert!(super::compare_cache::is_stale(&nothing));
}

#[test]
fn vault_task_compare_requires_unlock() {
    let fx = changed();
    withdraw_session(&fx.site_a.id);
    let error = compare_job_in(
        &fx.store,
        &local_resolve,
        &fx.job,
        &[],
        &AtomicBool::new(false),
    )
    .expect_err("a locked vault is not compared");
    assert_eq!(locked_site(&error), Some(fx.site_a.id.as_str()), "{error}");
    // Locking the key side ends the comparison made before.
    publish_session(Arc::clone(&fx.session));
    let view = fx.compare(&[]);
    assert!(!view.items.is_empty());
    fx.session.lock();
    let after = super::compare::compare_snapshot(&fx.job.id, view.revision)
        .expect_err("the lock ended the comparison");
    assert!(locked_site(&after).is_some() || super::compare_cache::is_stale(&after));
    assert!(unlocked_session(&fx.site_a.id).is_none());
}

#[test]
fn vault_task_compare_uses_other_side_key() {
    let fx = changed();
    withdraw_session(&fx.site_a.id);
    // Only side B is unlocked: the same vault, so the same key.
    let session_b = VaultSession::new(
        &fx.site_b.id,
        crate::cryptomator::Masterkey::from_raw(
            &fx.session.with_key(|key| Ok(*key.raw())).unwrap(),
        ),
        fx.session.config().clone(),
    );
    publish_session(Arc::clone(&session_b));
    let view = fx.compare(&[]);
    assert_eq!(item(&view, "/neu-a.txt").group, CompareGroup::NewA);
    let state = cache().get(&fx.job.id).expect("kept comparison");
    assert_eq!(state.site, fx.site_b.id);
    withdraw_session(&fx.site_b.id);
}

#[test]
fn vault_task_compare_size_is_cleartext() {
    let fx = Fx::new();
    fx.write(&fx.root_a, "/gross.bin", &pattern(70_000));
    let view = fx.compare(&[]);
    let entry = item(&view, "/gross.bin");
    assert_eq!(entry.a.size, Some(70_000), "the size of the cleartext");
    assert!(entry.a.exists && !entry.b.exists);
    assert_eq!(entry.b.size, None);
    assert_eq!(entry.a.location, "Tresor A");
    assert!(entry.a.mtime_ms.is_some());
    // The cipher file is larger than the cleartext.
    let cipher = stored(&fx.root_a)
        .values()
        .map(Vec::len)
        .max()
        .expect("files");
    assert!(cipher > 70_000);
}

#[test]
fn vault_task_compare_merge_limit_refused() {
    let fx = Fx::new();
    let big_a = vec![b'a'; 600 * 1024];
    let big_b = vec![b'b'; 600 * 1024];
    // Equal sizes: the sync tells the sides apart by their times, so a
    // pause separates every pair of writes.
    tick();
    fx.write(&fx.root_a, "/gross.txt", &big_a);
    tick();
    fx.write(&fx.root_b, "/gross.txt", &big_b);
    fx.write(&fx.root_a, "/klein.txt", b"von A");
    tick();
    fx.write(&fx.root_b, "/klein.txt", b"von B");
    let view = fx.compare(&[CompareGroup::Conflict]);
    let large = item(&view, "/gross.txt");
    assert!(large.actions.contains(&ItemAction::KeepA));
    assert!(large.actions.contains(&ItemAction::KeepBoth));
    assert!(
        !large.actions.contains(&ItemAction::Merge),
        "above the limit only a side can be kept: {:?}",
        large.actions
    );
    assert!(super::compare_merge::compare_text_job(&fx.job.id, &large.id).is_err());
    assert!(super::compare_merge::merge_apply_item(
        &fx.job.id,
        &large.id,
        "x",
        &AtomicBool::new(false)
    )
    .is_err());
    let small = item(&view, "/klein.txt");
    assert!(small.actions.contains(&ItemAction::Merge));
    let (a, b) = super::compare_merge::compare_text_job(&fx.job.id, &small.id).expect("texts");
    assert_eq!((a.as_str(), b.as_str()), ("von A", "von B"));
    assert_eq!(super::compare_merge::MERGE_MAX_BYTES, 512 * 1024);
}
