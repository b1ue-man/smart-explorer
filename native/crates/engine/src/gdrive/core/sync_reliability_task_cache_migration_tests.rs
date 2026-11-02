use super::api::FOLDER_MIME;
use super::sync_conflict_task_fixture::{filter, MIME};
use super::sync_reliability_task_fixture::{
    assert_complete, contains_bytes, options, DriveFixture,
};
use crate::bisync::{self, Direction};
use crate::vfs::{self, Backend};
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::AtomicBool;

#[test]
fn sync_reliability_task_identity_global_hint_survives_other_root_account_cache() {
    let mut f = DriveFixture::new("Notebook");
    let original = f.root_object_id();
    let note = f.write_file("note.md", b"historical Notebook bytes");
    f.drive
        .insert("other-root", "Other", "root", FOLDER_MIME, b"");
    f.drive.insert(
        "other-healthy",
        "healthy.txt",
        "other-root",
        MIME,
        b"other root seed",
    );
    let legacy = f.directory.path().join("legacy-path-cache.json");
    let account = f.directory.path().join("account-path-cache.json");
    super::cache::save_to_path(
        &legacy,
        HashMap::from([("Notebook".into(), original.clone())]),
        HashMap::from([("Notebook".into(), FOLDER_MIME.into())]),
    )
    .unwrap();
    let legacy_bytes = std::fs::read(&legacy).unwrap();
    let opts = options(Direction::BtoA);

    // This is the first new connection after the update. It writes the new
    // account cache without ever projecting or choosing the old Notebook root.
    let other = f
        .fresh_backend("Other")
        .test_with_loaded_caches(account.clone(), &legacy);
    let other_path = f.directory.path().join("other-local");
    std::fs::create_dir(&other_path).unwrap();
    let other_root = other_path.to_string_lossy().replace('\\', "/");
    let other_local = vfs::LocalBackend::new(&other_root);
    let other_seed = bisync::run(
        &other_local,
        &other_root,
        &other,
        "/Other",
        opts,
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    );
    assert_complete(&other_seed);
    assert_eq!(
        std::fs::read(other_path.join("healthy.txt")).unwrap(),
        b"other root seed"
    );
    other.persist_path_cache_checked().unwrap();
    assert!(account.is_file());
    let record = other
        .binding_store
        .read(&other.drive_account_key, &f.drive.root_id)
        .unwrap();
    assert!(record.by_segment("Notebook").is_none());
    drop(other);

    f.drive
        .insert("later-notebook", "Notebook", "root", FOLDER_MIME, b"");
    f.drive.change(
        "later-notebook",
        json!({"modifiedTime":"2035-01-01T00:00:00Z"}),
    );
    f.drive.insert(
        "later-note",
        "note.md",
        "later-notebook",
        MIME,
        b"replacement candidate bytes",
    );
    f.backend = f
        .fresh_backend("Notebook")
        .test_with_loaded_caches(account.clone(), &legacy);
    assert_eq!(
        f.backend.captured_legacy_id("Notebook").unwrap().as_deref(),
        Some(original.as_str())
    );
    let seed = f.run(opts);
    assert_complete(&seed);
    assert_eq!(f.local_bytes("note.md"), b"historical Notebook bytes");
    assert_eq!(
        f.backend.item_id(&f.root).unwrap().as_deref(),
        Some(original.as_str())
    );
    f.assert_noop(opts, &seed);
    f.backend.persist_path_cache_checked().unwrap();

    f.backend = f
        .fresh_backend("Notebook")
        .test_with_loaded_caches(account, &legacy);
    f.drive.insert(
        &note,
        "note.md",
        &original,
        MIME,
        b"changed historical Notebook",
    );
    let changed = f.run(opts);
    assert_complete(&changed);
    assert_eq!(changed.state, seed.state);
    assert_eq!(f.local_bytes("note.md"), b"changed historical Notebook");
    assert_eq!(
        f.backend.item_id(&f.root).unwrap().as_deref(),
        Some(original.as_str())
    );
    assert!(contains_bytes(
        &bisync::versions_dir(&changed.state.as_ref().unwrap().pair_id),
        b"historical Notebook bytes"
    ));
    assert_eq!(f.drive.bytes("later-note"), b"replacement candidate bytes");
    assert_eq!(f.drive.bytes("other-healthy"), b"other root seed");
    assert_eq!(std::fs::read(&legacy).unwrap(), legacy_bytes);
    assert_eq!(f.folder_posts(), 0);
    f.assert_noop(opts, &changed);
}

#[test]
fn sync_reliability_task_identity_account_cache_alone_never_proves_an_old_plain_root() {
    let mut f = DriveFixture::new("Notebook");
    let original = f.root_object_id();
    let note = f.write_file("note.md", b"first candidate bytes");
    f.drive
        .insert("other-notebook", "Notebook", "root", FOLDER_MIME, b"");
    f.drive.insert(
        "other-note",
        "note.md",
        "other-notebook",
        MIME,
        b"second candidate bytes",
    );
    f.write_local("note.md", b"previous old job bytes");
    let meta = f.local.stat(&format!("{}/note.md", f.local_root)).unwrap();
    let sig = bisync::Sig {
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        hash: 0,
    };
    let baseline = bisync::Baseline::from([("note.md".into(), (Some(sig), Some(sig)))]);
    let old_path = bisync::baseline_path(&bisync::pair_id_for(
        &f.local,
        &f.local_root,
        &f.backend,
        &f.root,
    ));
    bisync::save_baseline(&old_path, &baseline).unwrap();
    let old_bytes = std::fs::read(&old_path).unwrap();
    let account = f.directory.path().join("account-path-cache.json");
    let legacy = f.directory.path().join("missing-global-cache.json");
    super::cache::save_to_path(
        &account,
        HashMap::from([("Notebook".into(), original.clone())]),
        HashMap::from([("Notebook".into(), FOLDER_MIME.into())]),
    )
    .unwrap();
    f.backend = f
        .fresh_backend("Notebook")
        .test_with_loaded_caches(account.clone(), &legacy);
    assert!(f.backend.captured_legacy_id("Notebook").unwrap().is_none());
    let opts = options(Direction::BtoA);
    let blocked = f.run(opts);
    assert!(!blocked.errors.is_empty());
    assert_eq!(f.mutations(), 0);
    assert_eq!(f.local_bytes("note.md"), b"previous old job bytes");
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);

    let parent = f
        .fresh_backend("")
        .test_with_loaded_caches(account.clone(), &legacy);
    let browser = parent.list_dir("/").unwrap();
    assert_eq!(browser.iter().filter(|entry| entry.is_dir).count(), 2);
    let records = f.registry_bytes();
    assert_eq!(
        vfs::sync_stat(&f.backend, &f.root).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    let mut writer = f.backend.open_write("/Notebook/attempt.md").unwrap();
    writer.write_all(b"must remain protected").unwrap();
    assert_eq!(
        writer.flush().unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    drop(writer);
    assert_eq!(f.registry_bytes(), records);
    assert_eq!(f.mutations(), 0);
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);

    let picked = browser
        .iter()
        .find(|entry| entry.id.as_deref() == Some(original.as_str()))
        .unwrap();
    assert!(picked.name.contains("[drive-id"));
    f.root = format!("/{}", picked.name);
    f.backend = f
        .fresh_backend(&f.root)
        .test_with_loaded_caches(account.clone(), &legacy);
    super::sync_reliability_task_fixture::resolve_initial_note(&f, &f.backend, &f.root, opts);
    let selected = f.run(opts);
    assert_complete(&selected);
    assert_eq!(f.local_bytes("note.md"), b"first candidate bytes");
    assert!(contains_bytes(
        &bisync::versions_dir(&selected.state.as_ref().unwrap().pair_id),
        b"previous old job bytes"
    ));
    f.backend.persist_path_cache_checked().unwrap();
    f.backend = f
        .fresh_backend(&f.root)
        .test_with_loaded_caches(account, &legacy);
    f.drive.insert(
        &note,
        "note.md",
        &original,
        MIME,
        b"changed exact picker tree",
    );
    let changed = f.run(opts);
    assert_complete(&changed);
    assert_eq!(changed.state, selected.state);
    assert_eq!(f.local_bytes("note.md"), b"changed exact picker tree");
    assert!(contains_bytes(
        &bisync::versions_dir(&changed.state.as_ref().unwrap().pair_id),
        b"first candidate bytes"
    ));
    assert_eq!(
        f.backend.item_id(&f.root).unwrap().as_deref(),
        Some(original.as_str())
    );
    assert_eq!(f.drive.bytes("other-note"), b"second candidate bytes");
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
    assert_eq!(f.folder_posts(), 0);
    f.assert_noop(opts, &changed);
}
