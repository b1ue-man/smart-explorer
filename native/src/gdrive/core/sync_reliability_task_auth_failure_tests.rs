use super::sync_reliability_task_auth_fixture::{oauth_credentials as credentials, token_answer};
use super::sync_reliability_task_fixture::{
    assert_complete, options, state_baseline, DriveFixture,
};
use super::task_drive::drive_error;
use super::task_http::Answer;
use crate::bisync::Direction;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[test]
fn sync_reliability_task_resume_temporary_about_and_invalid_grant_preserve_baseline_then_recover() {
    for invalid in [false, true] {
        let fault = Arc::new(AtomicBool::new(false));
        let f = DriveFixture::with_handler("Job", {
            let fault = fault.clone();
            move |_, request| {
                if request.path() == "/oauth/token" && invalid && fault.load(Ordering::SeqCst) {
                    return Some(Answer::status(400, json!({"error":"invalid_grant"})));
                }
                if let Some(answer) = token_answer(request) {
                    return Some(answer);
                }
                if request.path() == "/drive/v3/about" && !invalid && fault.load(Ordering::SeqCst) {
                    return Some(
                        drive_error(503, "backendError", "about temporarily unavailable")
                            .header("Retry-After", "0"),
                    );
                }
                None
            }
        });
        f.write_file("note.md", b"confirmed before token check");
        let opts = options(Direction::BtoA);
        let seed = f.run(opts);
        assert_complete(&seed);
        let seed = f.assert_noop(opts, &seed);
        let records = f.registry_bytes();
        let _credentials = credentials(&f);
        f.write_file("note.md", b"after restored token check");
        f.backend.tokens_guard().unwrap().expires_at = 0;
        fault.store(true, Ordering::SeqCst);
        let failed = f.run(opts);
        assert!(!failed.errors.is_empty());
        assert_eq!(f.local_bytes("note.md"), b"confirmed before token check");
        assert_eq!(state_baseline(seed.state.as_ref().unwrap()), seed.baseline);
        assert_eq!(f.registry_bytes(), records);
        fault.store(false, Ordering::SeqCst);
        let recovered = f.run(opts);
        assert_complete(&recovered);
        assert_eq!(recovered.state, seed.state);
        assert_eq!(f.local_bytes("note.md"), b"after restored token check");
        assert!(super::sync_reliability_task_fixture::contains_bytes(
            &crate::bisync::versions_dir(&recovered.state.as_ref().unwrap().pair_id),
            b"confirmed before token check"
        ));
        assert_eq!(f.registry_bytes(), records);
        f.assert_noop(opts, &recovered);
    }
}
