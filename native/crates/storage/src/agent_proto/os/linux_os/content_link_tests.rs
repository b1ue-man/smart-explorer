//! A content search never enters or reads a link: a linked folder and a
//! linked file below the root stay unsearched, while the same content in an
//! ordinary file is found.
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use crate::agent_proto::{search_local_tree, ContentEvent, ContentQuery};

#[test]
fn content_search_task_links_are_never_entered_or_read() {
    let outside = tempfile::tempdir().expect("temp dir");
    std::fs::write(outside.path().join("secret.txt"), "needle outside\n").unwrap();
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    std::fs::write(root.join("plain.txt"), "needle inside\n").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("linked-folder")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        root.join("linked-file.txt"),
    )
    .unwrap();

    let events = Mutex::new(Vec::new());
    let emit = |event: ContentEvent| {
        events.lock().unwrap().push(event);
        true
    };
    let mut query = ContentQuery::text("needle");
    query.hidden = true;
    let summary = search_local_tree(root, &query, &emit, &AtomicBool::new(false)).unwrap();
    let files: Vec<String> = events
        .into_inner()
        .unwrap()
        .into_iter()
        .filter_map(|event| match event {
            ContentEvent::Hits { rel, .. } => Some(rel),
            _ => None,
        })
        .collect();
    assert_eq!(files, ["plain.txt"]);
    assert_eq!(summary.files_searched, 1);
    assert_eq!(summary.skipped, 0);
}
