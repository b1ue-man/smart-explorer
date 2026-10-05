use super::api::FOLDER_MIME;
use super::sync_reliability_task_fixture::{
    assert_complete, contains_bytes, options, DriveFixture,
};
use crate::bisync::{self, Direction};
use crate::vfs::{self, Backend};
use serde_json::json;

#[test]
fn sync_reliability_task_notebook_exact_candidates_preview_change_restart_and_noop() {
    let mut f = DriveFixture::new("Notebook");
    let notebook = f.root_object_id();
    for (id, title) in [("wrong-lower", "notebook"), ("wrong-upper", "NOTEBOOK")] {
        f.drive.insert(id, title, "root", FOLDER_MIME, b"");
        f.drive.insert(
            &format!("{id}-file"),
            "wrong.txt",
            id,
            super::sync_conflict_task_fixture::MIME,
            b"foreign case tree",
        );
    }
    let app_id = f.write_file(".obsidian/app.json", br#"{"theme":"dark"}"#);
    f.write_file(".obsidian/plugins/notes/data.json", br#"{"enabled":true}"#);
    f.write_file("welcome.md", b"Notebook content\n");
    let opts = options(Direction::BtoA);
    let preview = f.preview(opts);
    assert!(preview.error.is_none(), "{:?}", preview.error);
    assert!(preview.blocked.is_none() && preview.conflicts.is_empty());
    assert!(preview.actions.len() >= 3);
    assert_eq!(f.mutations(), 0);
    let seed = f.run(opts);
    assert_complete(&seed);
    assert_eq!(f.local_bytes(".obsidian/app.json"), br#"{"theme":"dark"}"#);
    assert_eq!(
        f.local_bytes(".obsidian/plugins/notes/data.json"),
        br#"{"enabled":true}"#
    );
    assert_eq!(f.local_bytes("welcome.md"), b"Notebook content\n");
    assert!(!std::path::Path::new(&f.local_root)
        .join("wrong.txt")
        .exists());
    assert!(seed.baseline.contains_key(".obsidian/app.json"));
    assert_eq!(
        vfs::sync_stat(&f.backend, &f.root).unwrap().id.as_deref(),
        Some(notebook.as_str())
    );
    assert_eq!(
        f.folder_posts(),
        0,
        "existing Notebook and descendants are reused"
    );
    f.assert_noop(opts, &seed);

    let identity = f.backend.state_identity();
    f.backend = f.fresh_backend("Notebook");
    assert_eq!(f.backend.state_identity(), identity);
    f.drive.change(
        "wrong-lower",
        json!({"modifiedTime":"2031-10-04T00:00:00Z"}),
    );
    assert_eq!(
        f.write_file(".obsidian/app.json", br#"{"theme":"light"}"#),
        app_id
    );
    let changed = f.run(opts);
    assert_complete(&changed);
    assert_eq!(changed.state, seed.state);
    assert_eq!(f.local_bytes(".obsidian/app.json"), br#"{"theme":"light"}"#);
    assert!(contains_bytes(
        &bisync::versions_dir(&changed.state.as_ref().unwrap().pair_id),
        br#"{"theme":"dark"}"#
    ));
    assert_eq!(f.drive.named("root", "Notebook").len(), 1);
    assert_eq!(f.folder_posts(), 0);
    f.assert_noop(opts, &changed);
}

#[test]
fn sync_reliability_task_notebook_valid_legacy_id_wins_after_sibling_mtime_change() {
    let mut f = DriveFixture::new("Notebook");
    let original = f.root_object_id();
    let file = f.write_file(".obsidian/appearance.json", b"original Notebook");
    f.backend = f.historical_cache_backend("Notebook", "Notebook", &original);
    f.drive
        .insert("later-notebook", "Notebook", "root", FOLDER_MIME, b"");
    f.drive.change(
        "later-notebook",
        json!({"modifiedTime":"2035-01-01T00:00:00Z"}),
    );
    f.drive.insert(
        "later-content",
        "wrong.txt",
        "later-notebook",
        super::sync_conflict_task_fixture::MIME,
        b"different folder",
    );
    let opts = options(Direction::BtoA);
    let seed = f.run(opts);
    assert_complete(&seed);
    assert_eq!(
        f.local_bytes(".obsidian/appearance.json"),
        b"original Notebook"
    );
    assert_eq!(
        f.backend.item_id(&f.root).unwrap().as_deref(),
        Some(original.as_str())
    );
    let bindings = f.registry_bytes();
    assert!(!bindings.is_empty());
    f.backend = f.fresh_backend("Notebook");
    f.drive.insert(
        &file,
        "appearance.json",
        f.drive.object(&file).unwrap()["parents"][0]
            .as_str()
            .unwrap(),
        super::sync_conflict_task_fixture::MIME,
        b"updated original Notebook",
    );
    let changed = f.run(opts);
    assert_complete(&changed);
    assert_eq!(changed.state, seed.state);
    assert_eq!(
        f.local_bytes(".obsidian/appearance.json"),
        b"updated original Notebook"
    );
    assert_eq!(
        f.backend.item_id(&f.root).unwrap().as_deref(),
        Some(original.as_str())
    );
    assert_eq!(f.registry_bytes(), bindings);
    assert!(contains_bytes(
        &bisync::versions_dir(&changed.state.as_ref().unwrap().pair_id),
        b"original Notebook"
    ));
    assert_eq!(f.drive.named("root", "Notebook").len(), 2);
    assert_eq!(f.folder_posts(), 0);
    f.assert_noop(opts, &changed);
}
