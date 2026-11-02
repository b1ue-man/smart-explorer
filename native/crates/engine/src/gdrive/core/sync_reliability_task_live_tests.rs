//! C10 deliberately fails when real authorization is unavailable. Neither
//! Drive's API base nor Google's OAuth endpoint is replaced in this test.
use super::sync_conflict_task_fixture::filter;
use super::sync_reliability_task_auth_fixture::CloudCredentials;
use super::sync_reliability_task_fixture::{contains_bytes, options};
use super::sync_reliability_task_live_fixture::LiveTree;
use super::GDriveBackend;
use crate::bisync::{self, Direction, Outcome};
use crate::vfs::{self, Backend, LocalBackend};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;

fn required(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            panic!("C10 AUTHORIZATION BLOCKER: required environment value {name} is missing")
        })
}

pub(super) fn checked<T>(result: io::Result<T>, action: &str) -> T {
    result.unwrap_or_else(|error| {
        panic!(
            "C10 {action} failed ({:?}); no credential values are logged",
            error.kind()
        )
    })
}

fn connect(root: &str) -> GDriveBackend {
    let backend = GDriveBackend::connect(root).unwrap_or_else(|_| {
        panic!("C10 AUTHORIZATION BLOCKER: regular stored OAuth connection failed")
    });
    assert_eq!(backend.api_base.as_ref(), super::api::API);
    backend
}

fn complete(out: &Outcome) {
    assert!(
        out.errors.is_empty(),
        "C10 sync has errors; no OAuth/HTTP error bodies are logged"
    );
    assert!(out.conflicts.is_empty() && out.omissions.is_empty() && out.deferred.is_empty());
    assert!(out.blocked.is_none() && out.stopped.is_none() && !out.busy && !out.canceled);
}

fn run(local: &LocalBackend, local_root: &str, remote: &GDriveBackend, root: &str) -> Outcome {
    bisync::run(
        local,
        local_root,
        remote,
        root,
        options(Direction::BtoA),
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    )
}

fn bytes(backend: &GDriveBackend, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut reader = checked(backend.open_read(path), "read owned file");
    checked(reader.read_to_end(&mut bytes), "read complete owned file");
    bytes
}

#[test]
fn sync_reliability_task_live_drive_notebook_ids_refresh_restart_backups_and_noop() {
    let config = crate::cloud::ClientConfig {
        client_id: required("SE_DRIVE_TEST_CLIENT_ID"),
        client_secret: std::env::var("SE_DRIVE_TEST_CLIENT_SECRET").unwrap_or_default(),
    };
    let refresh = required("SE_DRIVE_TEST_REFRESH_TOKEN");
    let _credentials = CloudCredentials::install(config, &refresh, None);
    let mut tree = LiveTree::create(connect(""));
    let own_root = tree.root_id.clone();
    let notebook = tree.folder(&own_root, "Notebook");
    let case_folder = tree.folder(&own_root, "notebook");
    let obsidian = tree.folder(&notebook, ".obsidian");
    let plugins = tree.folder(&obsidian, "plugins");
    let notes = tree.folder(&plugins, "notes");
    let app = tree.file(&obsidian, "app.json", b"{\"seed\":true}");
    let plugin = tree.file(&notes, "data.json", b"{\"plugin\":\"notes\"}");
    let welcome = tree.file(&notebook, "welcome.md", b"real Notebook welcome");
    let wrong = tree.file(&case_folder, "wrong.md", b"separate real lowercase tree");
    let red = tree.file(&obsidian, "appearance.json", b"red real variant");
    let blue = tree.file(&obsidian, "appearance.json", b"blue chosen real variant");
    let same_a = tree.file(&notebook, "same.txt", b"same real variants");
    let same_b = tree.file(&notebook, "same.txt", b"same real variants");
    let root = format!("{}/Notebook", tree.root_path);
    let backend = connect(&root);
    assert_eq!(
        checked(
            vfs::sync_stat(&backend, &root),
            "resolve exact Notebook root"
        )
        .id
        .as_deref(),
        Some(notebook.as_str())
    );
    checked(
        backend.persist_path_cache_checked(),
        "persist regular Notebook locator hint",
    );
    let identity = backend.state_identity();
    let directory = tempfile::tempdir().unwrap();
    let local_root = directory.path().join("local");
    std::fs::create_dir(&local_root).unwrap();
    let local_path = local_root.to_string_lossy().replace('\\', "/");
    let local = LocalBackend::new(&local_path);
    std::fs::create_dir_all(local_root.join(".obsidian")).unwrap();
    std::fs::write(local_root.join("same.txt"), b"same real variants").unwrap();
    std::fs::write(
        local_root.join(".obsidian/appearance.json"),
        b"local preserved seed",
    )
    .unwrap();
    let preview = bisync::preview(
        &local,
        &local_path,
        &backend,
        &root,
        options(Direction::BtoA),
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    );
    assert!(preview.error.is_none(), "C10 preview failed");
    assert_eq!(preview.conflicts.len(), 1);
    let state = preview.state.unwrap();
    checked(
        bisync::resolve_recorded(
            &local,
            &local_path,
            &backend,
            &root,
            &preview.conflicts[0],
            false,
            Some(&blue),
            &state,
            &AtomicBool::new(false),
            |_| {},
        ),
        "choose exact real file variant",
    );
    assert_eq!(
        std::fs::read(local_root.join(".obsidian/appearance.json")).unwrap(),
        b"blue chosen real variant"
    );
    assert_eq!(
        checked(tree.backend.object_json(&red), "read discarded variant")["trashed"],
        true
    );
    assert_eq!(
        checked(tree.backend.object_json(&blue), "read retained variant")["trashed"],
        false
    );
    let pair = bisync::pair_id_for(&local, &local_path, &backend, &root);
    assert!(contains_bytes(
        &bisync::versions_dir(&pair),
        b"local preserved seed"
    ));
    assert!(contains_bytes(
        &bisync::versions_dir(&pair),
        b"red real variant"
    ));
    let seed = run(&local, &local_path, &backend, &root);
    complete(&seed);
    for (rel, expected, id) in [
        (
            ".obsidian/app.json",
            b"{\"seed\":true}".as_slice(),
            app.as_str(),
        ),
        (
            ".obsidian/plugins/notes/data.json",
            b"{\"plugin\":\"notes\"}".as_slice(),
            plugin.as_str(),
        ),
        (
            "welcome.md",
            b"real Notebook welcome".as_slice(),
            welcome.as_str(),
        ),
        (
            ".obsidian/appearance.json",
            b"blue chosen real variant".as_slice(),
            blue.as_str(),
        ),
    ] {
        assert_eq!(std::fs::read(local_root.join(rel)).unwrap(), expected);
        let locator = checked(
            vfs::sync_path(&backend, &root, rel),
            "build literal file locator",
        );
        assert_eq!(
            checked(backend.item_id(&locator), "read captured published ID").as_deref(),
            Some(id)
        );
        assert!(seed.baseline.contains_key(rel));
    }
    assert!(!local_root.join("wrong.md").exists());
    let equal = checked(
        tree.backend.collect_files(&notebook, Some("same.txt")),
        "collect real equal variants",
    );
    assert_eq!(equal.len(), 1);
    assert!([same_a.as_str(), same_b.as_str()].contains(&equal[0]["id"].as_str().unwrap()));

    let duplicate = tree.folder(&own_root, "Notebook");
    let duplicate_note = tree.file(&duplicate, "other.md", b"independent real duplicate tree");
    let siblings = checked(
        vfs::list_dir_tolerant(&tree.backend, &tree.root_path),
        "project real duplicate folders",
    );
    assert!(siblings.omitted.is_empty());
    let projected: BTreeMap<_, _> = siblings
        .entries
        .into_iter()
        .filter(|entry| entry.is_dir)
        .map(|entry| (entry.id.unwrap(), entry.name))
        .collect();
    assert_eq!(projected.len(), 3);
    assert_eq!(projected[&notebook], "Notebook");
    assert_eq!(projected[&case_folder], "notebook");
    assert_ne!(projected[&duplicate], "Notebook");
    let duplicate_root = checked(
        vfs::sync_child_path(&tree.backend, &tree.root_path, &projected[&duplicate]),
        "pick real duplicate",
    );
    let duplicate_backend = connect(&duplicate_root);
    let duplicate_local = directory.path().join("duplicate-local");
    std::fs::create_dir(&duplicate_local).unwrap();
    let duplicate_path = duplicate_local.to_string_lossy().replace('\\', "/");
    let duplicate_target = LocalBackend::new(&duplicate_path);
    let duplicate_seed = run(
        &duplicate_target,
        &duplicate_path,
        &duplicate_backend,
        &duplicate_root,
    );
    complete(&duplicate_seed);
    assert_eq!(
        std::fs::read(duplicate_local.join("other.md")).unwrap(),
        b"independent real duplicate tree"
    );
    assert_eq!(
        checked(
            duplicate_backend.item_id(&duplicate_root),
            "verify selected duplicate ID"
        )
        .as_deref(),
        Some(duplicate.as_str())
    );
    let duplicate_noop = run(
        &duplicate_target,
        &duplicate_path,
        &duplicate_backend,
        &duplicate_root,
    );
    complete(&duplicate_noop);
    assert_eq!(duplicate_noop.stats.bytes, 0);
    assert_eq!(
        duplicate_noop.stats.a_to_b + duplicate_noop.stats.b_to_a + duplicate_noop.stats.deleted,
        0
    );
    assert_eq!(duplicate_noop.baseline, duplicate_seed.baseline);

    let app_path = checked(
        vfs::sync_path(&backend, &root, ".obsidian/app.json"),
        "build app update locator",
    );
    let mut writer = checked(backend.open_write(&app_path), "open exact real app update");
    checked(
        writer.write_all(b"{\"changed\":true}"),
        "write real app update",
    );
    checked(writer.flush(), "confirm real app update");
    drop(writer);
    let restarted = connect(&root); // regular cloud store and same saved locator
    assert_eq!(restarted.state_identity(), identity);
    restarted.tokens_guard().unwrap().expires_at = 0; // real regular OAuth renewal, same principal
    assert_eq!(
        checked(
            vfs::sync_stat(&restarted, &root),
            "verify after real token rotation"
        )
        .id
        .as_deref(),
        Some(notebook.as_str())
    );
    let changed = run(&local, &local_path, &restarted, &root);
    complete(&changed);
    assert_eq!(changed.state, seed.state);
    assert_eq!(
        std::fs::read(local_root.join(".obsidian/app.json")).unwrap(),
        b"{\"changed\":true}"
    );
    assert_eq!(bytes(&restarted, &app_path), b"{\"changed\":true}");
    assert_eq!(
        checked(restarted.item_id(&app_path), "verify app ID after restart").as_deref(),
        Some(app.as_str())
    );
    assert!(contains_bytes(
        &bisync::versions_dir(&pair),
        b"{\"seed\":true}"
    ));
    let before = tree.snapshot();
    let noop = run(&local, &local_path, &restarted, &root);
    complete(&noop);
    assert_eq!(noop.stats.bytes, 0);
    assert_eq!(
        noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted,
        0
    );
    assert_eq!(noop.baseline, changed.baseline);
    assert_eq!(
        tree.snapshot(),
        before,
        "No-op leaves every captured real ID unchanged"
    );
    assert_eq!(
        checked(
            tree.backend.collect_files(&own_root, None),
            "verify own folder set"
        )
        .len(),
        3
    );
    assert_eq!(
        bytes(
            &tree.backend,
            &format!("{}/notebook/wrong.md", tree.root_path)
        ),
        b"separate real lowercase tree"
    );
    assert_eq!(
        checked(tree.backend.object_json(&wrong), "verify lowercase ID")["trashed"],
        false
    );
    assert_eq!(
        checked(
            tree.backend.object_json(&duplicate_note),
            "verify duplicate note ID"
        )["trashed"],
        false
    );
    super::sync_reliability_task_fixture::assert_persisted(&noop);
    eprintln!("C10 real ID oracles: Notebook={notebook}, duplicate={duplicate}, selected_file={blue}, app={app}");
    checked(tree.cleanup(), "clean captured own Drive IDs");
}
