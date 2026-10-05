use super::sync_reliability_task_fixture::{
    assert_complete, contains_bytes, options, DriveFixture,
};
use super::task_drive::drive_error;
use super::task_http::{multipart, Answer};
use crate::bisync::{self, Direction};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[test]
fn sync_reliability_task_resume_lost_acks_reconcile_exact_ids_without_duplicate_creation() {
    for operation in ["folder", "multipart", "rename", "replace", "trash"] {
        let armed = Arc::new(AtomicBool::new(false));
        let committed = Arc::new(Mutex::new(None::<String>));
        let f = DriveFixture::with_handler("Notebook", {
            let (armed, committed) = (armed.clone(), committed.clone());
            move |drive, request| {
                if !armed.load(Ordering::SeqCst) {
                    return None;
                }
                let metadata: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                let matches = match operation {
                    "folder" => request.method == "POST" && request.path() == "/drive/v3/files",
                    "multipart" => {
                        request.method == "POST"
                            && request.query("uploadType").as_deref() == Some("multipart")
                    }
                    "rename" => {
                        request.method == "PATCH"
                            && request.path().starts_with("/drive/v3/files/")
                            && metadata["name"] == "new.md"
                    }
                    "replace" => {
                        request.method == "PUT"
                            && request.path().starts_with("/upload/session/")
                            && !request.body.is_empty()
                    }
                    _ => request.method == "PATCH" && metadata["trashed"] == true,
                };
                if matches && armed.swap(false, Ordering::SeqCst) {
                    let answer = drive.answer(request); // commit before losing the acknowledgement
                    assert_eq!(answer.status, 200);
                    let json: Value = serde_json::from_slice(&answer.body).unwrap();
                    *committed.lock().unwrap() = Some(json["id"].as_str().unwrap().into());
                    return Some(if operation == "multipart" || operation == "folder" {
                        Answer::disconnect()
                    } else {
                        drive_error(503, "backendError", "acknowledgement lost after commit")
                    });
                }
                None
            }
        });
        let original = f.write_file(".obsidian/app.json", b"original confirmed bytes");
        let seed = f.run(options(Direction::BtoA));
        assert_complete(&seed);
        let seed = f.assert_noop(options(Direction::BtoA), &seed);
        f.write_local(
            ".obsidian/app.json",
            b"replacement acknowledged by exact ID",
        );
        f.write_local(".obsidian/plugins/notes/data.json", b"new plugin tree");
        f.write_local(".obsidian/new.md", b"new absent destination");
        armed.store(true, Ordering::SeqCst);
        let recovered = f.run(options(Direction::AtoB));
        assert_complete(&recovered);
        assert!(
            !armed.load(Ordering::SeqCst),
            "{operation} committed through the real publisher"
        );
        let exact = committed.lock().unwrap().clone().unwrap();
        assert!(
            f.server
                .requests()
                .iter()
                .any(|r| r.method == "GET" && r.path() == format!("/drive/v3/files/{exact}")),
            "uncertain ACK is settled by captured ID"
        );
        assert_eq!(recovered.state, seed.state);
        assert_eq!(
            f.read_file(".obsidian/app.json"),
            b"replacement acknowledged by exact ID"
        );
        assert_eq!(
            f.drive.bytes(&original),
            b"replacement acknowledged by exact ID"
        );
        assert_eq!(
            f.read_file(".obsidian/plugins/notes/data.json"),
            b"new plugin tree"
        );
        assert_eq!(f.read_file(".obsidian/new.md"), b"new absent destination");
        let pair = bisync::pair_id_for(&f.local, &f.local_root, &f.backend, &f.root);
        assert!(contains_bytes(
            &bisync::versions_dir(&pair),
            b"original confirmed bytes"
        ));
        let requests = f.server.requests();
        let creates = requests
            .iter()
            .filter(|request| {
                request.method == "POST"
                    && (request.path() == "/drive/v3/files"
                        || request.query("uploadType").as_deref() == Some("multipart"))
            })
            .map(|request| {
                if request.query("uploadType").as_deref() == Some("multipart") {
                    multipart(request).0["id"].as_str().unwrap().to_string()
                } else {
                    serde_json::from_slice::<Value>(&request.body).unwrap()["id"]
                        .as_str()
                        .unwrap()
                        .to_string()
                }
            })
            .collect::<Vec<_>>();
        let unique: std::collections::HashSet<_> = creates.iter().collect();
        assert_eq!(
            unique.len(),
            creates.len(),
            "never replay an uncertain create"
        );
        assert!(f.backend.owned_stages_guard().unwrap().is_empty());
        let posts = f.folder_posts();
        f.assert_noop(options(Direction::Both), &recovered);
        assert_eq!(f.folder_posts(), posts);
    }
}
