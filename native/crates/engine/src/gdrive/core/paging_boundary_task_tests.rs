//! Explicit identity and restart scripts at the shared listing boundaries.
use super::super::gui_task_http::Step;
use super::scan_task_tests::{child, listing, root, ROOT_ID};
use super::{names, step, Fixture, Reply};
use crate::vfs::Backend;
use serde_json::{json, Value};
use std::io::{self, Write};

pub(super) fn uncached_paged_markers_and_reloaded_cache() {
    let raw = "I/O\\Gerät's Daten";
    let first = child(raw, "abcdefgh1", ROOT_ID, false);
    let second = child(raw, "abcdefgh2", ROOT_ID, false);
    let literal = "literal [drive-id abc]";
    let fixture = Fixture::new(vec![
        root(),
        listing(ROOT_ID, None, Some(raw),
            Reply::Json(json!({"files": [first], "nextPageToken": "next"}))),
        listing(ROOT_ID, Some("next"), Some(raw),
            Reply::Json(json!({"files": [second.clone()]}))),
        super::scan_task_tests::identity("/drive/v3/files/abcdefgh2", &second),
        listing(ROOT_ID, None, Some(literal), Reply::Json(json!({
            "files": [child(literal, "literal-id", ROOT_ID, false)]
        }))),
    ]);
    let backend = fixture.backend();
    let path = format!("{} [drive-id abcdefgh2]", names::encode(raw));
    assert_eq!(backend.resolve(&path).unwrap(), "abcdefgh2");
    backend.untrusted_guard().unwrap().insert(path.clone());
    assert_eq!(backend.resolve(&path).unwrap(), "abcdefgh2");
    assert_eq!(backend.resolve(&names::encode(literal)).unwrap(), "literal-id");
    assert!(backend.state_identity().starts_with("gdrive:path-v2:"));
    let requests = fixture.finish();
    assert_eq!(requests.len(), 5);
    super::assert_query(&requests[1], ROOT_ID, raw);
    assert!(requests[2].target.contains("pageToken=next"));
    super::assert_query(&requests[4], ROOT_ID, literal);
}

fn incomplete_steps(name: Option<&str>, page: &Value) -> Vec<Step> {
    let mut steps = Vec::new();
    // api.rs retains six bounded complete-read attempts. Each incomplete
    // attempt before the final one probes the parent before narrowing again.
    for attempt in 0..6 {
        steps.push(listing(ROOT_ID, None, name, Reply::Json(page.clone()))
            .query("corpora", (attempt > 0).then(|| "user".into())));
        if attempt < 5 {
            steps.push(step("GET", "/drive/v3/files/scan-root-id",
                Reply::Json(json!({"id": ROOT_ID})))
                .query("fields", Some("id,driveId".into())));
        }
    }
    steps
}

pub(super) fn repeated_or_incomplete_pages_fail_closed() {
    for incomplete in [false, true] {
        let body = json!({"files": [], "nextPageToken": "repeat", "incompleteSearch": incomplete});
        let mut steps = vec![root()];
        if incomplete {
            steps.extend(incomplete_steps(Some("I/O"), &body));
        } else {
            for _ in 0..6 {
                steps.push(listing(ROOT_ID, None, Some("I/O"), Reply::Json(body.clone())));
                steps.push(listing(ROOT_ID, Some("repeat"), Some("I/O"), Reply::Json(body.clone())));
            }
        }
        let fixture = Fixture::new(steps);
        let error = fixture.backend().find_child("root", "I%2FO").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let expected = if incomplete { "search is incomplete" } else { "repeated a listing page token" };
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(fixture.finish().len(), if incomplete { 12 } else { 13 });
    }
}

fn boundary_steps(page: &Value) -> Vec<Step> {
    let mut steps = vec![root()];
    for name in [None, Some("I/O"), Some("I/O")] {
        if page["incompleteSearch"].as_bool() == Some(true) {
            steps.extend(incomplete_steps(name, page));
        } else {
            steps.push(listing(ROOT_ID, None, name, Reply::Json(page.clone())));
        }
    }
    steps
}

pub(super) fn optional_empty_pages_agree_at_read_and_mutation_boundaries() {
    for page in [
        json!({}),
        json!({"files": null}),
        json!({"files": []}),
        json!({"files": [], "nextPageToken": null, "incompleteSearch": null}),
    ] {
        let fixture = Fixture::new(boundary_steps(&page));
        let backend = fixture.backend();
        assert!(backend.list_dir("/").unwrap().is_empty());
        assert_eq!(backend.find_child("root", "I%2FO").unwrap(), None);
        assert!(backend.named_objects("root", "I/O").unwrap().is_empty());
        assert_eq!(fixture.finish().len(), 4);
    }
    for page in [
        json!(null),
        json!([]),
        json!({"files": {}}),
        json!({"files": ""}),
        json!({"nextPageToken": 3}),
        json!({"incompleteSearch": "false"}),
        json!({"incompleteSearch": true}),
        json!({"error": {"message": "failed"}}),
    ] {
        let fixture = Fixture::new(boundary_steps(&page));
        let backend = fixture.backend();
        assert_eq!(backend.list_dir("/").unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(backend.find_child("root", "I%2FO").unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(backend.named_objects("root", "I/O").unwrap_err().kind(), io::ErrorKind::InvalidData);
        let expected = if page["incompleteSearch"].as_bool() == Some(true) { 34 } else { 4 };
        assert_eq!(fixture.finish().len(), expected);
    }
    // Invalid absence must not reserve an ID or start create/upload mutation.
    let fixture = Fixture::new(vec![
        root(),
        listing(ROOT_ID, None, Some("I/O"), Reply::Json(json!({"files": false}))),
    ]);
    let mut writer = fixture.backend().open_write("I%2FO").unwrap();
    writer.write_all(b"abc").unwrap();
    assert_eq!(writer.flush().unwrap_err().kind(), io::ErrorKind::InvalidData);
    drop(writer);
    assert_eq!(fixture.finish().len(), 2);
}
