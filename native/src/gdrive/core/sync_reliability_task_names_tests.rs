use super::api::FOLDER_MIME;
use super::sync_conflict_task_fixture::{filter, Fixture, FILE, MIME};
use super::sync_reliability_task_fixture::{assert_complete, options, read, DriveFixture};
use crate::bisync::{self, Direction};
use crate::vfs::{self, Backend};
use serde_json::json;
use std::sync::atomic::AtomicBool;

#[test]
fn sync_reliability_task_names_file_variant_choice_preserves_ids_backups_and_baseline() {
    let f = Fixture::new(Some(b"local seed"), &[("choice-a", b"red remote"), ("choice-b", b"blue remote")]);
    let preview = f.preview();
    assert!(preview.error.is_none());
    assert_eq!(preview.conflicts.len(), 1);
    assert_eq!(f.mutation_count(), 0);
    let state = preview.state.unwrap();
    bisync::resolve_recorded(&f.local, &f.root, &f.remote, "/", &preview.conflicts[0], false,
        Some("choice-b"), &state, &AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(f.local_content(), b"blue remote");
    assert_eq!(f.remote_content(), b"blue remote");
    assert_eq!(f.drive.named("root", FILE)[0]["id"], "choice-b");
    f.assert_backed_up(b"local seed");
    f.assert_backed_up(b"red remote");
    let seed = f.run(options(Direction::Both));
    assert_complete(&seed);
    assert!(seed.baseline.contains_key(FILE));
    f.drive.insert("choice-b", FILE, "root", MIME, b"changed selected ID");
    let changed = f.run(options(Direction::BtoA));
    assert_complete(&changed);
    assert_eq!(f.local_content(), b"changed selected ID");
    assert_eq!(f.drive.named("root", FILE)[0]["id"], "choice-b");
    f.assert_backed_up(b"blue remote");
    let mutations = f.mutation_count();
    let noop = f.run(options(Direction::Both));
    assert_complete(&noop);
    assert_eq!(noop.stats.bytes, 0);
    assert_eq!(noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted, 0);
    assert_eq!(noop.baseline, changed.baseline);
    assert_eq!(f.mutation_count(), mutations);
    super::sync_reliability_task_fixture::assert_persisted(&noop);
}


#[test]
fn sync_reliability_task_names_reserved_alias_collision_is_partial_then_recovers() {
    let f = DriveFixture::new("Job");
    let parent = f.root_object_id();
    for id in ["canonical", "older12345"] {
        f.drive.insert(id, "Tree", &parent, FOLDER_MIME, b"");
        f.drive.insert(&format!("{id}-file"), "keep.txt", id, MIME, id.as_bytes());
    }
    let aliases = f.aliases(&f.root);
    let alias = &aliases["older12345"];
    let rel = format!("{alias}/keep.txt");
    let opts = options(Direction::BtoA);
    let seed = f.run(opts);
    assert_complete(&seed);
    f.drive.insert("literal-collision", alias, &parent, MIME, b"literal collision bytes");
    f.write_file("healthy.txt", b"independent completion");
    let partial = f.run(opts);
    assert!(partial.errors.is_empty(), "{:?}", partial.errors);
    assert!(partial.omissions.protects(&rel));
    assert_eq!(partial.baseline.get(&rel), seed.baseline.get(&rel));
    assert_eq!(super::sync_reliability_task_fixture::state_baseline(seed.state.as_ref().unwrap()).get(&rel),
        seed.baseline.get(&rel));
    assert_eq!(f.local_bytes(&rel), b"older12345");
    assert_eq!(f.local_bytes("healthy.txt"), b"independent completion");
    assert_eq!(f.drive.bytes("literal-collision"), b"literal collision bytes");
    assert_eq!(f.aliases(&f.root), aliases);
    f.drive.change("literal-collision", json!({"trashed":true}));
    let recovered = f.run(opts);
    assert_complete(&recovered);
    assert_eq!(f.local_bytes(&rel), b"older12345");
    f.assert_noop(opts, &recovered);
}


#[cfg(windows)]
#[test]
fn sync_reliability_task_names_windows_targetlimits_preserve_protected_counterparts() {
    let f = DriveFixture::new("Job");
    f.write_file("Notebook/note.md", b"previous valid tree");
    f.write_file("healthy.txt", b"before");
    let opts = options(Direction::BtoA);
    let seed = f.run(opts);
    assert_complete(&seed);
    f.write_file("notebook/note.md", b"separate literal case tree");
    for title in ["CON", "colon:name", "back\\slash", "tail "] {
        f.write_file(title, b"unrepresentable retained remotely");
    }
    f.write_file("healthy.txt", b"after target protection");
    let partial = f.run(opts);
    assert!(partial.errors.is_empty(), "{:?}", partial.errors);
    for rel in ["Notebook/note.md", "notebook/note.md", "CON", "colon:name", "back\\slash", "tail "] {
        assert!(partial.omissions.protects(rel), "Windows target must protect {rel}");
    }
    assert_eq!(partial.baseline.get("Notebook/note.md"), seed.baseline.get("Notebook/note.md"));
    assert_eq!(super::sync_reliability_task_fixture::state_baseline(seed.state.as_ref().unwrap()).get("Notebook/note.md"),
        seed.baseline.get("Notebook/note.md"));
    assert_eq!(f.local_bytes("Notebook/note.md"), b"previous valid tree");
    assert_eq!(f.local_bytes("healthy.txt"), b"after target protection");
    assert!(super::sync_reliability_task_fixture::contains_bytes(
        &bisync::versions_dir(&partial.state.as_ref().unwrap().pair_id), b"before"));
    assert_eq!(f.read_file("notebook/note.md"), b"separate literal case tree");
    let parent = f.root_object_id();
    for title in ["notebook", "CON", "colon:name", "back\\slash", "tail "] {
        let objects = f.drive.named(&parent, title);
        assert_eq!(objects.len(), 1);
        f.drive.change(objects[0]["id"].as_str().unwrap(), json!({"trashed":true}));
    }
    let recovered = f.run(opts);
    assert_complete(&recovered);
    assert_eq!(f.local_bytes("Notebook/note.md"), b"previous valid tree");
    assert_eq!(f.local_bytes("healthy.txt"), b"after target protection");
    f.assert_noop(opts, &recovered);
}


#[test]
fn sync_reliability_task_names_exact_picker_rename_and_trash_touch_one_folder_id() {
    let f = DriveFixture::new("Job");
    let parent = f.root_object_id();
    for id in ["tree-a", "tree-b"] {
        f.drive.insert(id, "Tree", &parent, FOLDER_MIME, b"");
        f.drive.insert(&format!("{id}-file"), "note.md", id, MIME, id.as_bytes());
    }
    let aliases = f.aliases(&f.root);
    let selected = vfs::sync_child_path(&f.backend, &f.root, &aliases["tree-b"]).unwrap();
    let destination = vfs::sync_child_path(&f.backend, &f.root, "Renamed").unwrap();
    f.backend.rename(&selected, &destination).unwrap();
    assert_eq!(f.drive.object("tree-a").unwrap()["name"], "Tree");
    assert_eq!(f.drive.object("tree-b").unwrap()["name"], "Renamed");
    assert_eq!(read(&f.backend, &format!("{destination}/note.md")), b"tree-b");
    assert!(f.backend.stat(&selected).is_err(), "old locator cannot select the other tree");
    let seed = f.run(options(Direction::BtoA));
    assert_complete(&seed);
    f.backend.remove_dir(&destination).unwrap();
    assert_eq!(f.drive.object("tree-b").unwrap()["trashed"], true);
    assert_eq!(f.drive.object("tree-a").unwrap()["trashed"], false);
    assert_eq!(f.drive.bytes("tree-a-file"), b"tree-a");
    let changed = f.run(options(Direction::BtoA));
    assert_complete(&changed);
    assert_eq!(f.local_bytes("Tree/note.md"), b"tree-a");
    assert!(!std::path::Path::new(&f.local_root).join("Renamed/note.md").exists());
    assert!(super::sync_reliability_task_fixture::contains_bytes(
        &bisync::versions_dir(&changed.state.as_ref().unwrap().pair_id), b"tree-b"));
    f.assert_noop(options(Direction::BtoA), &changed);
}
