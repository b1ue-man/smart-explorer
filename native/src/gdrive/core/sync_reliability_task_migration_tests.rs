use super::api::FOLDER_MIME;
use super::sync_conflict_task_fixture::{filter, MIME};
use super::sync_reliability_task_fixture::{
    assert_complete, options, state_baseline, DriveFixture,
};
use crate::bisync::{self, Direction};
use crate::vfs::{self, Backend};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::atomic::AtomicBool;

#[test]
fn sync_reliability_task_identity_unknown_old_root_stays_protected_until_exact_picker() {
    let f = DriveFixture::new("Notebook");
    let original = f.root_object_id();
    f.drive.insert(
        "original-note",
        "note.md",
        &original,
        MIME,
        b"old root candidate",
    );
    f.drive
        .insert("other-notebook", "Notebook", "root", FOLDER_MIME, b"");
    f.drive.insert(
        "other-note",
        "note.md",
        "other-notebook",
        MIME,
        b"other root candidate",
    );
    f.write_local("note.md", b"previous old job bytes");
    let meta = f.local.stat(&format!("{}/note.md", f.local_root)).unwrap();
    let old_sig = bisync::Sig {
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        hash: 0,
    };
    let old_baseline = bisync::Baseline::from([("note.md".into(), (Some(old_sig), Some(old_sig)))]);
    let old_pair = bisync::pair_id_for(&f.local, &f.local_root, &f.backend, &f.root);
    let old_path = bisync::baseline_path(&old_pair);
    bisync::save_baseline(&old_path, &old_baseline).unwrap();
    let old_bytes = std::fs::read(&old_path).unwrap();
    let opts = options(Direction::BtoA);
    let old_identity = f.backend.state_identity();
    let first = f.run(opts);
    assert!(!first.errors.is_empty());
    assert_eq!(f.mutations(), 0);
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
    assert_eq!(f.local_bytes("note.md"), b"previous old job bytes");
    assert!(f
        .backend
        .stat(&f.root)
        .unwrap_err()
        .to_string()
        .contains("exact folder"));

    let parent = f.fresh_backend("");
    let parent_local = f.directory.path().join("parent-local");
    std::fs::create_dir(&parent_local).unwrap();
    let parent_root = parent_local.to_string_lossy().replace('\\', "/");
    let local = vfs::LocalBackend::new(&parent_root);
    let parents = bisync::run(
        &local,
        &parent_root,
        &parent,
        "/",
        opts,
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    );
    assert_complete(&parents);
    assert_eq!(parents.baseline.len(), 2);
    let browser = parent.list_dir("/").unwrap();
    assert_eq!(browser.iter().filter(|entry| entry.is_dir).count(), 2);
    for entry in browser.iter().filter(|entry| entry.is_dir) {
        let path = format!("/{}/note.md", entry.name);
        let logical = vfs::sync_stat(&parent, &format!("/{}", entry.name))
            .unwrap()
            .name;
        let bytes = super::sync_reliability_task_fixture::read(&parent, &path);
        assert_eq!(
            std::fs::read(parent_local.join(logical).join("note.md")).unwrap(),
            bytes
        );
    }
    let records = f.registry_bytes();
    let old = f.fresh_backend(&f.root);
    let error = vfs::sync_stat(&old, &f.root).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(old.state_identity(), old_identity);
    assert_eq!(f.registry_bytes(), records);
    assert_eq!(f.local_bytes("note.md"), b"previous old job bytes");
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
    let mutations = f.mutations();
    let mut writer = old.open_write("/Notebook/attempt.md").unwrap();
    writer.write_all(b"must not choose an old root").unwrap();
    assert_eq!(
        writer.flush().unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    drop(writer);
    assert_eq!(
        f.mutations(),
        mutations,
        "normal writer cannot bypass root provenance"
    );
    assert_eq!(f.registry_bytes(), records);
    assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
    assert_eq!(f.drive.bytes("original-note"), b"old root candidate");
    assert_eq!(f.drive.bytes("other-note"), b"other root candidate");

    let picked = browser
        .iter()
        .find(|entry| entry.id.as_deref() == Some(original.as_str()))
        .unwrap();
    assert!(picked.name.contains("[drive-id"));
    let root = format!("/{}", picked.name); // existing picker result; no new job codec
    let exact = f.fresh_backend(&root);
    super::sync_reliability_task_fixture::resolve_initial_note(&f, &exact, &root, opts);
    let selected = bisync::run(
        &f.local,
        &f.local_root,
        &exact,
        &root,
        opts,
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    );
    assert_complete(&selected);
    assert_eq!(f.local_bytes("note.md"), b"old root candidate");
    assert!(super::sync_reliability_task_fixture::contains_bytes(
        &bisync::versions_dir(&selected.state.as_ref().unwrap().pair_id),
        b"previous old job bytes"
    ));
    assert_eq!(
        exact.item_id(&root).unwrap().as_deref(),
        Some(original.as_str())
    );
    let restarted = f.fresh_backend(&root);
    f.drive.insert(
        "original-note",
        "note.md",
        &original,
        MIME,
        b"changed selected root",
    );
    let changed = bisync::run(
        &f.local,
        &f.local_root,
        &restarted,
        &root,
        opts,
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    );
    assert_complete(&changed);
    assert_eq!(changed.state, selected.state);
    assert_eq!(f.local_bytes("note.md"), b"changed selected root");
    assert!(super::sync_reliability_task_fixture::contains_bytes(
        &bisync::versions_dir(&changed.state.as_ref().unwrap().pair_id),
        b"old root candidate"
    ));
    let mutations = f.mutations();
    let noop = bisync::run(
        &f.local,
        &f.local_root,
        &restarted,
        &root,
        opts,
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    );
    assert_complete(&noop);
    assert_eq!(noop.stats.bytes, 0);
    assert_eq!(
        noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted,
        0
    );
    assert_eq!(noop.baseline, changed.baseline);
    assert_eq!(f.mutations(), mutations);
    assert_eq!(f.folder_posts(), 0);
    assert_eq!(f.drive.bytes("other-note"), b"other root candidate");
    super::sync_reliability_task_fixture::assert_persisted(&noop);
}

#[test]
fn sync_reliability_task_identity_corrupt_unknown_and_originless_records_preserve_state() {
    let f = DriveFixture::new("Job");
    let id = f.write_file("note.md", b"confirmed remote bytes");
    let opts = options(Direction::AtoB);
    let seed = f.run(options(Direction::BtoA));
    assert_complete(&seed);
    let seed = f.assert_noop(options(Direction::BtoA), &seed);
    let records = f.registry_bytes();
    let (relative, bytes) = records
        .iter()
        .find(|(_, bytes)| {
            let disk: Value = serde_json::from_slice(bytes).unwrap();
            disk["bindings"]["parent"] == f.drive.root_id
        })
        .unwrap();
    let path = f.directory.path().join("bindings").join(relative);
    let mut unknown: Value = serde_json::from_slice(bytes).unwrap();
    unknown["bindings"]["version"] = json!(999);
    f.write_local("note.md", b"pending local change");
    for corrupt in [
        serde_json::to_vec(&unknown).unwrap(),
        b"{truncated".to_vec(),
    ] {
        std::fs::write(&path, &corrupt).unwrap();
        let mutations = f.mutations();
        let blocked = f.run(opts);
        assert!(!blocked.errors.is_empty());
        assert_eq!(f.mutations(), mutations);
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        assert_eq!(f.drive.bytes(&id), b"confirmed remote bytes");
        assert_eq!(state_baseline(seed.state.as_ref().unwrap()), seed.baseline);
    }
    std::fs::write(&path, bytes).unwrap();
    let recovered = f.run(opts);
    assert_complete(&recovered);
    assert_eq!(f.drive.bytes(&id), b"pending local change");
    assert!(super::sync_reliability_task_fixture::contains_bytes(
        &bisync::versions_dir(&recovered.state.as_ref().unwrap().pair_id),
        b"confirmed remote bytes"
    ));
    f.assert_noop(opts, &recovered);

    // The unreleased v1 format lacked provenance. Keep its checksum/IDs, but
    // do not let format migration confirm a formerly unknown ambiguous root.
    let mut disk: Value = serde_json::from_slice(bytes).unwrap();
    let mut bindings: super::sync_bindings::FolderBindings =
        serde_json::from_value(disk["bindings"].clone()).unwrap();
    for binding in &mut bindings.folders {
        binding.evidence = None;
    }
    use sha2::{Digest, Sha256};
    disk["checksum"] = json!(Sha256::digest(serde_json::to_vec(&bindings).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>());
    disk["bindings"] = serde_json::to_value(bindings).unwrap();
    let originless = serde_json::to_vec(&disk).unwrap();
    std::fs::write(&path, &originless).unwrap();
    f.drive
        .insert("ambiguous-job", "Job", "root", FOLDER_MIME, b"");
    let old = f.fresh_backend("Job");
    assert_eq!(
        vfs::sync_stat(&old, "/Job").unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(&path).unwrap(), originless);
    assert_eq!(f.drive.bytes(&id), b"pending local change");
}
