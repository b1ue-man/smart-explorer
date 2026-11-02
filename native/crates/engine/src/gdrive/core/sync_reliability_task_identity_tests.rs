use super::api::FOLDER_MIME;
use super::sync_conflict_task_fixture::MIME;
use super::sync_reliability_task_fixture::{
    assert_complete, options, state_baseline, DriveFixture,
};
use super::task_drive::drive_error;
use super::task_http::Answer;
use super::GDriveBackend;
use crate::bisync::Direction;
use crate::vfs::{self, Backend};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Duration;

#[test]
fn sync_reliability_task_identity_binding_survives_failures_restart_and_cache_clear() {
    for failure in ["permission", "temporary", "disconnect"] {
        let deny = Arc::new(AtomicBool::new(false));
        let mut f = DriveFixture::with_handler("Job", {
            let deny = deny.clone();
            move |_, request| {
                if deny.load(Ordering::SeqCst)
                    && request.method == "GET"
                    && request.path() == "/drive/v3/files/original-folder"
                {
                    return Some(match failure {
                        "permission" => {
                            drive_error(403, "insufficientFilePermissions", "verification denied")
                        }
                        "temporary" => drive_error(503, "backendError", "verification unavailable")
                            .header("Retry-After", "0"),
                        _ => Answer::disconnect(),
                    });
                }
                None
            }
        });
        let parent = f.root_object_id();
        for (id, bytes) in [
            ("original-folder", b"original tree".as_slice()),
            ("newer-folder", b"other tree".as_slice()),
        ] {
            f.drive.insert(id, "Notebook", &parent, FOLDER_MIME, b"");
            f.drive
                .insert(&format!("{id}-note"), "note.md", id, MIME, bytes);
        }
        f.backend = f.historical_cache_backend("Job", "Job/Notebook", "original-folder");
        f.drive.change(
            "newer-folder",
            json!({"modifiedTime":"2030-01-01T00:00:00Z"}),
        );
        let aliases = f.aliases(&f.root);
        assert_eq!(aliases["original-folder"], "Notebook");
        let opts = options(Direction::BtoA);
        let seed = f.run(opts);
        assert_complete(&seed);
        let seed = f.assert_noop(opts, &seed);
        let records = f.registry_bytes();
        assert!(!records.is_empty());
        f.backend = f.fresh_backend(&f.root); // cache cleared; durable identity survives
        deny.store(true, Ordering::SeqCst);
        let mutations = f.mutations();
        let partial = f.run(opts);
        assert!(!partial.errors.is_empty() || partial.omissions.protects("Notebook/note.md"));
        assert_eq!(f.local_bytes("Notebook/note.md"), b"original tree");
        assert_eq!(
            partial.baseline.get("Notebook/note.md"),
            seed.baseline.get("Notebook/note.md")
        );
        assert_eq!(state_baseline(seed.state.as_ref().unwrap()), seed.baseline);
        assert_eq!(f.registry_bytes(), records);
        assert_eq!(f.mutations(), mutations);
        deny.store(false, Ordering::SeqCst);
        f.drive.insert(
            "original-folder-note",
            "note.md",
            "original-folder",
            MIME,
            b"recovered exact ID",
        );
        let recovered = f.run(opts);
        assert_complete(&recovered);
        assert_eq!(f.local_bytes("Notebook/note.md"), b"recovered exact ID");
        assert_eq!(f.aliases(&f.root), aliases);
        assert_eq!(f.registry_bytes(), records);
        assert_eq!(f.drive.bytes("newer-folder-note"), b"other tree");
        assert!(super::sync_reliability_task_fixture::contains_bytes(
            &crate::bisync::versions_dir(&recovered.state.as_ref().unwrap().pair_id),
            b"original tree"
        ));
        assert_eq!(f.folder_posts(), 0);
        f.assert_noop(opts, &recovered);
    }
}

#[test]
fn sync_reliability_task_identity_concurrent_binding_and_accounts_remain_independent() {
    let f = DriveFixture::new("Job");
    let parent = f.root_object_id();
    for id in ["tree-a", "tree-b"] {
        f.drive.insert(id, "Tree", &parent, FOLDER_MIME, b"");
        f.drive
            .insert(&format!("{id}-note"), "note.md", id, MIME, id.as_bytes());
    }
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let (backend, root, barrier) =
                (f.fresh_backend(&f.root), f.root.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                vfs::list_dir_tolerant(&backend, &root)
                    .unwrap()
                    .entries
                    .into_iter()
                    .map(|entry| (entry.id.unwrap(), entry.name))
                    .collect::<BTreeMap<_, _>>()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results[0], results[1]);
    let aliases = f.aliases(&f.root);
    assert_eq!(aliases, results[0]);
    let opts = options(Direction::BtoA);
    let seed = f.run(opts);
    assert_complete(&seed);
    f.assert_noop(opts, &seed);
    f.drive
        .change("tree-b", json!({"modifiedTime":"2030-01-01T00:00:00Z"}));
    f.drive.insert(
        "tree-b-note",
        "note.md",
        "tree-b",
        MIME,
        b"stable second tree",
    );
    let changed = f.run(opts);
    assert_complete(&changed);
    assert_eq!(f.aliases(&f.root), aliases);
    assert_eq!(
        f.local_bytes(&format!("{}/note.md", aliases["tree-b"])),
        b"stable second tree"
    );
    assert!(super::sync_reliability_task_fixture::contains_bytes(
        &crate::bisync::versions_dir(&changed.state.as_ref().unwrap().pair_id),
        b"tree-b"
    ));
    let other = DriveFixture::new("Job");
    other.write_file("note.md", b"different account");
    let same_profile = GDriveBackend::test_backend_with_identity(
        &other.server.api_base(),
        Duration::from_secs(2),
        Some(f.directory.path().join("pending")),
        Some(f.directory.path().join("bindings")),
        &other.drive.permission_id,
        "Job",
    );
    assert_ne!(same_profile.state_identity(), f.backend.state_identity());
    assert_eq!(
        super::sync_reliability_task_fixture::read(&same_profile, "/Job/note.md"),
        b"different account"
    );
    assert_eq!(f.aliases(&f.root), aliases);
    assert_eq!(f.drive.bytes("tree-a-note"), b"tree-a");
    assert!(f.registry_bytes().len() >= 3);
    assert_eq!(f.folder_posts(), 0);
    f.assert_noop(opts, &changed);
}
