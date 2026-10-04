use super::sync_reliability_task_fixture::{assert_complete, contains_bytes, options, state_baseline, DriveFixture};
use super::sync_reliability_task_auth_fixture::{token_answer, oauth_credentials as credentials, oauth_posts};
use super::task_drive::drive_error;
use super::task_http::Answer;
use crate::bisync::{self, Direction};
use crate::vfs::{self, Backend};
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

#[test]
fn sync_reliability_task_resume_early_401_rotates_tokens_and_resumes_the_same_pair() {
    for stream in [false, true] {
        let reject = Arc::new(AtomicBool::new(false));
        let f = DriveFixture::with_handler("Notebook", {
            let reject = reject.clone();
            move |_, request| {
                if let Some(answer) = token_answer(request) { return Some(answer); }
                let media = request.query("alt").as_deref() == Some("media");
                if request.method == "GET" && request.path() != "/drive/v3/about" && media == stream
                    && request.header("authorization") == Some("Bearer test-token")
                    && reject.swap(false, Ordering::SeqCst) {
                    return Some(drive_error(401, "authError", "access token expired early"));
                }
                None
            }
        });
        let id = f.write_file(".obsidian/app.json", b"before rotation");
        let opts = options(Direction::BtoA);
        let seed = f.run(opts);
        assert_complete(&seed);
        let seed = f.assert_noop(opts, &seed);
        let records = f.registry_bytes();
        let identity = f.backend.state_identity();
        let _credentials = credentials(&f);
        assert_eq!(f.write_file(".obsidian/app.json", b"same job after OAuth refresh"), id);
        reject.store(true, Ordering::SeqCst);
        let recovered = f.run(opts);
        assert_complete(&recovered);
        assert!(!reject.load(Ordering::SeqCst), "the real read boundary observed the 401");
        assert_eq!(oauth_posts(&f), 1);
        assert_eq!(crate::cloud::refresh_token_checked(crate::cloud::Provider::GDrive).unwrap().as_deref(),
            Some("rotated-fixture-refresh"));
        assert_eq!(f.local_bytes(".obsidian/app.json"), b"same job after OAuth refresh");
        assert_eq!(recovered.state, seed.state);
        assert_eq!(f.backend.state_identity(), identity);
        assert_eq!(f.registry_bytes(), records);
        assert_eq!(f.drive.bytes(&id), b"same job after OAuth refresh");
        let pair = bisync::pair_id_for(&f.local, &f.local_root, &f.backend, &f.root);
        assert!(contains_bytes(&bisync::versions_dir(&pair), b"before rotation"));
        assert_eq!(f.folder_posts(), 0);
        f.assert_noop(opts, &recovered);
    }
}


#[test]
fn sync_reliability_task_resume_concurrent_401_refreshes_once_and_principal_change_is_protected() {
    let active = Arc::new(AtomicBool::new(false));
    let principal = Arc::new(AtomicBool::new(false));
    let arrivals = Arc::new(AtomicUsize::new(0));
    let f = DriveFixture::with_handler("Job", {
        let (active, principal, arrivals) = (active.clone(), principal.clone(), arrivals.clone());
        move |drive, request| {
            if let Some(answer) = token_answer(request) { return Some(answer); }
            if request.path() == "/drive/v3/about" && principal.load(Ordering::SeqCst) {
                return Some(Answer::json(json!({"user":{"permissionId":"foreign-principal"}})));
            }
            if active.load(Ordering::SeqCst) && request.method == "GET"
                && request.header("authorization") == Some("Bearer test-token") {
                arrivals.fetch_add(1, Ordering::SeqCst);
                return Some(drive_error(401, "authError", "renewable token"));
            }
            let _ = drive;
            None
        }
    });
    let id = f.write_file("note.md", b"seed known account");
    let opts = options(Direction::BtoA);
    let seed = f.run(opts);
    assert_complete(&seed);
    let seed = f.assert_noop(opts, &seed);
    let records = f.registry_bytes();
    let _credentials = credentials(&f);
    active.store(true, Ordering::SeqCst);
    let barrier = Arc::new(Barrier::new(4));
    let workers: Vec<_> = (0..4).map(|_| {
        let (backend, barrier, root) = (f.backend.clone(), barrier.clone(), f.root.clone());
        std::thread::spawn(move || { barrier.wait(); vfs::sync_stat(&backend, &root).unwrap().id.unwrap() })
    }).collect();
    for worker in workers { assert_eq!(worker.join().unwrap(), f.root_object_id()); }
    assert!(arrivals.load(Ordering::SeqCst) > 0);
    assert_eq!(oauth_posts(&f), 1, "rejected old tokens share the coordinated refresh");
    assert_eq!(f.registry_bytes(), records);

    f.backend.tokens_guard().unwrap().access_token = "test-token".into();
    principal.store(true, Ordering::SeqCst);
    f.drive.insert(&id, "note.md", &f.root_object_id(), super::sync_conflict_task_fixture::MIME, b"pending same account");
    let mutations = f.mutations();
    let blocked = f.run(opts);
    assert!(blocked.errors.iter().any(|(_, error)| error.contains("Drive-Konto wurde gewechselt")));
    assert_eq!(f.local_bytes("note.md"), b"seed known account");
    assert_eq!(state_baseline(seed.state.as_ref().unwrap()), seed.baseline);
    assert_eq!(f.registry_bytes(), records);
    assert_eq!(f.mutations(), mutations + 1, "only the OAuth refresh was sent");
    principal.store(false, Ordering::SeqCst);
    let recovered = f.run(opts);
    assert_complete(&recovered);
    assert_eq!(recovered.state, seed.state);
    assert_eq!(f.local_bytes("note.md"), b"pending same account");
    assert!(contains_bytes(&bisync::versions_dir(&recovered.state.as_ref().unwrap().pair_id), b"seed known account"));
    assert_eq!(f.registry_bytes(), records);
    f.assert_noop(opts, &recovered);
}
