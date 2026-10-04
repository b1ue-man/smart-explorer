use super::sync_conflict_task_fixture::{Fixture, FILE};
use super::sync_reliability_task_fixture::{assert_complete, contains_bytes, options, state_baseline, DriveFixture};
use super::task_drive::drive_error;
use crate::bisync::{self, Direction, ResolvePhase};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[test]
fn sync_reliability_task_resume_quota_permissions_and_cancel_preserve_confirmed_bytes() {
    for reason in ["storageQuotaExceeded", "insufficientFilePermissions"] {
        let denied = Arc::new(AtomicBool::new(false));
        let f = DriveFixture::with_handler("Job", {
            let denied = denied.clone();
            move |_, request| {
                if denied.load(Ordering::SeqCst) && request.path().starts_with("/upload/") {
                    return Some(drive_error(403, reason, "destination currently refuses writes"));
                }
                None
            }
        });
        let id = f.write_file("note.md", b"before refused upload");
        let seed = f.run(options(Direction::BtoA));
        assert_complete(&seed);
        let seed = f.assert_noop(options(Direction::BtoA), &seed);
        f.write_local("note.md", b"same pair resumes upload");
        denied.store(true, Ordering::SeqCst);
        let refused = f.run(options(Direction::AtoB));
        assert!(!refused.errors.is_empty() || refused.stopped.is_some());
        assert_eq!(f.drive.bytes(&id), b"before refused upload");
        assert_eq!(state_baseline(seed.state.as_ref().unwrap()), seed.baseline);
        denied.store(false, Ordering::SeqCst);
        let recovered = f.run(options(Direction::AtoB));
        assert_complete(&recovered);
        assert_eq!(recovered.state, seed.state);
        assert_eq!(f.drive.bytes(&id), b"same pair resumes upload");
        assert!(contains_bytes(&bisync::versions_dir(&recovered.state.as_ref().unwrap().pair_id), b"before refused upload"));
        assert_eq!(f.drive.named(&f.root_object_id(), "note.md").len(), 1);
        f.assert_noop(options(Direction::Both), &recovered);
    }

    for phase in ["scan", "backup", "stage", "publish"] {
        let armed = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let f = DriveFixture::with_handler("Job", {
            let (armed, cancel) = (armed.clone(), cancel.clone());
            move |drive, request| {
                if !armed.load(Ordering::SeqCst) { return None; }
                let matches = match phase {
                    "scan" => request.method == "GET" && request.path() == "/drive/v3/files",
                    "backup" => request.query("alt").as_deref() == Some("media"),
                    "stage" => request.method == "POST" && request.query("uploadType").as_deref() == Some("multipart"),
                    _ => request.method == "PUT" && request.path().starts_with("/upload/session/") && !request.body.is_empty(),
                };
                if matches && armed.swap(false, Ordering::SeqCst) {
                    let answer = drive.answer(request);
                    cancel.store(true, Ordering::SeqCst); // normal engine cancellation token
                    return Some(answer);
                }
                None
            }
        });
        let id = f.write_file("note.md", b"before cancellation");
        let seed = f.run(options(Direction::BtoA));
        assert_complete(&seed);
        let seed = f.assert_noop(options(Direction::BtoA), &seed);
        f.write_local("note.md", b"after successful continuation");
        armed.store(true, Ordering::SeqCst);
        let interrupted = f.run_cancel(options(Direction::AtoB), &cancel);
        assert!(!armed.load(Ordering::SeqCst), "cancel reached {phase}");
        assert!(interrupted.canceled);
        if phase != "publish" {
            assert_eq!(f.drive.bytes(&id), b"before cancellation");
            assert_eq!(state_baseline(seed.state.as_ref().unwrap()), seed.baseline);
        } else { assert_eq!(f.drive.bytes(&id), b"after successful continuation"); }
        cancel.store(false, Ordering::SeqCst);
        let recovered = f.run(options(Direction::AtoB));
        assert_complete(&recovered);
        assert_eq!(recovered.state, seed.state);
        assert_eq!(f.drive.bytes(&id), b"after successful continuation");
        assert_eq!(f.drive.named(&f.root_object_id(), "note.md").len(), 1);
        let pair = bisync::pair_id_for(&f.local, &f.local_root, &f.backend, &f.root);
        assert!(contains_bytes(&bisync::versions_dir(&pair), b"before cancellation"));
        f.assert_noop(options(Direction::Both), &recovered);
    }
}


#[test]
fn sync_reliability_task_resume_partial_variant_publish_retains_recovery_then_converges() {
    let f = Fixture::new(Some(b"local retained choice"), &[("a", b"old A"), ("b", b"old B")]);
    let conflict = f.conflict();
    let cancel = AtomicBool::new(false);
    let failed = f.resolve(&conflict, true, None, &cancel, |phase| {
        if phase == ResolvePhase::Deleting { cancel.store(true, Ordering::SeqCst); }
    }).unwrap_err();
    assert_eq!(failed.kind(), std::io::ErrorKind::Interrupted);
    f.assert_backed_up(b"old A");
    f.assert_backed_up(b"old B");
    assert_eq!(f.drive.named("root", FILE).len(), 2);
    assert_eq!(f.local_content(), b"local retained choice");
    assert!(f.resolve(&conflict, true, None, &AtomicBool::new(false), |_| {}).is_err(),
        "stale conflict evidence never authorizes another cleanup");
    let recovered = f.run(options(Direction::Both));
    assert_complete(&recovered);
    assert_eq!(f.remote_content(), b"local retained choice");
    assert!(recovered.baseline.contains_key(FILE));
    let mutations = f.mutation_count();
    let noop = f.run(options(Direction::Both));
    assert_complete(&noop);
    assert_eq!(noop.stats.bytes, 0);
    assert_eq!(noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted, 0);
    assert_eq!(noop.baseline, recovered.baseline);
    assert_eq!(f.mutation_count(), mutations);
    super::sync_reliability_task_fixture::assert_persisted(&noop);
}
