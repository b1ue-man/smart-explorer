use super::sync_conflict_task_fixture::MIME;
use super::sync_reliability_task_fixture::{assert_complete, options, DriveFixture};
use super::task_drive::drive_error;
use super::task_http::Answer;
use crate::bisync::Direction;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn sync_reliability_task_identity_paging_restarts_complete_collection_and_converges() {
    for fault in ["rejected", "cyclic", "incomplete", "overlap"] {
        let phase = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(AtomicUsize::new(0));
        let f = DriveFixture::with_handler("Job", {
            let (phase, observed) = (phase.clone(), observed.clone());
            move |drive, request| {
                let parent = drive.named("root", "Job")[0]["id"].as_str().unwrap().to_string();
                let query = request.query("q").unwrap_or_default();
                if request.method != "GET" || request.path() != "/drive/v3/files"
                    || !query.starts_with(&format!("'{parent}' in parents")) || query.contains("name =")
                    || phase.load(Ordering::SeqCst) == 0 { return None; }
                observed.fetch_add(1, Ordering::SeqCst);
                let token = request.query("pageToken");
                let stage = phase.load(Ordering::SeqCst);
                let ghost = json!({"id":"discarded-ghost", "name":"ghost.md", "parents":[parent],
                    "mimeType":MIME, "trashed":false, "size":"4",
                    "md5Checksum":format!("{:x}",md5::compute(b"fake")), "modifiedTime":"2026-10-02T12:34:56Z"});
                if stage == 1 && token.is_none() {
                    phase.store(2, Ordering::SeqCst);
                    return Some(Answer::json(match fault {
                        "incomplete" => json!({"files":[ghost],"incompleteSearch":true}),
                        "overlap" => json!({"files":[],"nextPageToken":"empty","incompleteSearch":false}),
                        _ => json!({"files":[ghost],"nextPageToken":"fault-token","incompleteSearch":false}),
                    }));
                }
                if fault == "rejected" && stage == 2 && token.as_deref() == Some("fault-token") {
                    phase.store(3, Ordering::SeqCst);
                    return Some(drive_error(400, "invalid", "expired page token"));
                }
                if fault == "cyclic" && stage == 2 && token.as_deref() == Some("fault-token") {
                    phase.store(3, Ordering::SeqCst);
                    return Some(Answer::json(json!({"files":[],"nextPageToken":"fault-token","incompleteSearch":false})));
                }
                if fault == "incomplete" && stage == 2 {
                    assert_eq!(request.query("corpora").as_deref(), Some("user"));
                    phase.store(3, Ordering::SeqCst);
                }
                if fault == "overlap" && stage == 2 && token.as_deref() == Some("empty") {
                    let mut stale = drive.children(&parent)[0].clone();
                    stale["md5Checksum"] = json!("stale-checksum");
                    phase.store(3, Ordering::SeqCst);
                    return Some(Answer::json(json!({"files":[stale],"nextPageToken":"overlap","incompleteSearch":false})));
                }
                if fault == "overlap" && stage == 3 && token.as_deref() == Some("overlap") {
                    phase.store(4, Ordering::SeqCst);
                    return Some(Answer::json(json!({"files":drive.children(&parent),"incompleteSearch":false})));
                }
                None
            }
        });
        let a = f.write_file("a.md", b"seed a");
        for title in ["b.md", "c.md", "d.md"] { f.write_file(title, title.as_bytes()); }
        let opts = options(Direction::BtoA);
        let seed = f.run(opts);
        assert_complete(&seed);
        let seed = f.assert_noop(opts, &seed);
        assert_eq!(f.write_file("a.md", b"changed after seed"), a);
        let added = f.write_file("e.md", b"last page");
        phase.store(1, Ordering::SeqCst);
        let recovered = f.run(opts);
        assert_complete(&recovered);
        assert_eq!(recovered.state, seed.state);
        assert_eq!(f.local_bytes("a.md"), b"changed after seed");
        assert_eq!(f.local_bytes("e.md"), b"last page");
        assert_eq!(f.drive.bytes(&added), b"last page");
        assert!(super::sync_reliability_task_fixture::contains_bytes(
            &crate::bisync::versions_dir(&recovered.state.as_ref().unwrap().pair_id), b"seed a"));
        assert!(!recovered.baseline.contains_key("ghost.md"));
        assert!(!std::path::Path::new(&f.local_root).join("ghost.md").exists());
        assert!(observed.load(Ordering::SeqCst) >= 3);
        if fault == "overlap" {
            assert!(f.server.requests().iter().any(|r| r.path() == format!("/drive/v3/files/{a}")));
        }
        assert_eq!(f.folder_posts(), 0);
        f.assert_noop(opts, &recovered);
    }
}
