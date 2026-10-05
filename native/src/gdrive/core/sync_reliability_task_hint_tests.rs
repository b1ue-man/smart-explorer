use super::api::FOLDER_MIME;
use super::sync_conflict_task_fixture::MIME;
use super::sync_reliability_task_fixture::{assert_complete, options, DriveFixture};
use super::task_http::Answer;
use crate::bisync::Direction;
use crate::vfs::Backend;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn sync_reliability_task_identity_missing_valid_folder_retries_the_same_listing() {
    for legacy in ["none", "before-projection", "after-projection"] {
        let omit = Arc::new(AtomicBool::new(false));
        let lists = Arc::new(AtomicUsize::new(0));
        let f = DriveFixture::with_handler("Job", {
            let (omit, lists) = (omit.clone(), lists.clone());
            move |drive, request| {
                let parent = drive.named("root", "Job")[0]["id"]
                    .as_str()
                    .unwrap()
                    .to_string();
                let q = request.query("q").unwrap_or_default();
                if request.method == "GET"
                    && request.path() == "/drive/v3/files"
                    && q.starts_with(&format!("'{parent}' in parents"))
                    && !q.contains("name =")
                {
                    lists.fetch_add(1, Ordering::SeqCst);
                    if omit.swap(false, Ordering::SeqCst) {
                        let children: Vec<_> = drive
                            .children(&parent)
                            .into_iter()
                            .filter(|child| child["id"] != "valid-original")
                            .collect();
                        return Some(Answer::json(
                            json!({"files":children,"incompleteSearch":false}),
                        ));
                    }
                }
                None
            }
        });
        f.write_file("healthy.txt", b"seed independent");
        let opts = options(Direction::BtoA);
        let seed = f.run(opts);
        assert_complete(&seed);
        let seed = f.assert_noop(opts, &seed);
        let parent = f.root_object_id();
        f.drive
            .insert("valid-original", "Notebook", &parent, FOLDER_MIME, b"");
        f.drive.insert(
            "valid-note",
            "note.md",
            "valid-original",
            MIME,
            b"proved existing folder",
        );
        if legacy == "before-projection" {
            f.backend
                .test_capture_legacy_folder("Job/Notebook", "valid-original");
        } else {
            assert_eq!(f.aliases(&f.root)["valid-original"], "Notebook");
            if legacy == "after-projection" {
                f.backend
                    .test_capture_legacy_folder("Job/Notebook", "valid-original");
            }
        }
        f.drive
            .insert("newer-other", "Notebook", &parent, FOLDER_MIME, b"");
        f.drive.change(
            "newer-other",
            json!({"modifiedTime":"2030-01-01T00:00:00Z"}),
        );
        f.drive.insert(
            "other-note",
            "note.md",
            "newer-other",
            MIME,
            b"independent duplicate",
        );
        let before = lists.load(Ordering::SeqCst);
        omit.store(true, Ordering::SeqCst);
        let recovered = f.run(opts);
        assert_complete(&recovered);
        assert_eq!(recovered.state, seed.state);
        assert_eq!(f.local_bytes("Notebook/note.md"), b"proved existing folder");
        assert!(
            lists.load(Ordering::SeqCst) >= before + 2,
            "the same call retries a complete scan"
        );
        assert!(f
            .server
            .requests()
            .iter()
            .any(|r| r.path() == "/drive/v3/files/valid-original"));
        assert_eq!(f.folder_posts(), 0);
        if legacy != "none" {
            assert_eq!(
                f.fresh_backend("Job/Notebook")
                    .item_id("/Job/Notebook")
                    .unwrap()
                    .as_deref(),
                Some("valid-original"),
                "fresh old evidence supplements an equal projected binding"
            );
        }
        f.assert_noop(opts, &recovered);
    }
}
