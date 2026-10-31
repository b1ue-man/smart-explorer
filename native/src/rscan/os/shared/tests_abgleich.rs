//! Remote recursive view: whole-tree request, folder walk and work stealing.
//! Test names start with `abgleich_task_` (the remote task suite selects them).
use super::tests_abgleich_fixture::{
    count, parity_items, parity_listings, plain, row_keys, scan, vmeta, walk_entry, TreeBackend,
    Walk,
};
use crate::scanner::{RetentionHandle, ScanRetention};
use crate::types::FileEntry;
use crate::vfs::{HashWalkItem, HashWalkRequest, OmissionReason, VfsOmission};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn abgleich_task_whole_tree_rows_equal_the_folder_walk_rows() {
    let whole = TreeBackend::new(parity_listings(), Walk::Items(parity_items()), 2);
    let whole_scan = scan(whole.clone(), "/r", None, None);
    assert_eq!(count(&whole.walk_requests), 1, "one whole-tree request");
    assert_eq!(
        count(&whole.lists),
        0,
        "no folder listing with the whole tree"
    );
    let request = (*whole.last_request.lock().unwrap()).expect("request recorded");
    assert_eq!(
        request,
        HashWalkRequest {
            algorithm: None,
            min_bytes: 0,
            meta: true
        }
    );

    let folders = TreeBackend::new(parity_listings(), Walk::Unsupported, 2);
    let folder_scan = scan(folders.clone(), "/r", None, None);
    assert!(count(&folders.lists) > 0, "the folder walk lists folders");

    assert_eq!(whole_scan.rows.len(), 9, "root plus eight rows");
    assert_eq!(row_keys(&whole_scan.rows), row_keys(&folder_scan.rows));
    assert!(whole_scan.errors.is_empty() && whole_scan.failed.is_empty());
    assert!(whole_scan.done.is_some());
}

#[test]
fn abgleich_task_unsupported_whole_tree_falls_back_to_the_folder_walk() {
    let backend = TreeBackend::new(parity_listings(), Walk::Unsupported, 2);
    let scanned = scan(backend.clone(), "/r", None, None);
    assert_eq!(
        count(&backend.walk_requests),
        0,
        "no whole-tree request when the capability is missing"
    );
    assert!(count(&backend.lists) > 0);
    assert_eq!(scanned.rows.len(), 9);
    assert!(scanned.errors.is_empty());
}

#[test]
fn abgleich_task_whole_tree_failure_before_any_row_falls_back() {
    let backend = TreeBackend::new(parity_listings(), Walk::FailBefore, 2);
    let scanned = scan(backend.clone(), "/r", None, None);
    assert!(count(&backend.lists) > 0);
    assert_eq!(scanned.rows.len(), 9);
    assert!(scanned.errors.is_empty());
}

#[test]
fn abgleich_task_whole_tree_failure_after_rows_is_reported_and_keeps_the_rows() {
    let backend = TreeBackend::new(parity_listings(), Walk::ItemsThenFail(parity_items()), 2);
    let scanned = scan(backend.clone(), "/r", None, None);
    assert_eq!(
        count(&backend.lists),
        0,
        "no fallback after rows were shown"
    );
    assert_eq!(scanned.rows.len(), 9);
    assert_eq!(scanned.errors.len(), 1, "errors: {:?}", scanned.errors);
    assert!(scanned.errors[0].contains("unterbrochen"));
    assert!(scanned.done.is_some());
}

#[test]
fn abgleich_task_whole_tree_omissions_are_failed_paths_and_keep_their_siblings() {
    let mut items = parity_items();
    items.insert(
        3,
        HashWalkItem::Omitted(VfsOmission {
            rel: "docs/locked".into(),
            reason: OmissionReason::Unreadable,
            detail: "Zugriff verweigert".into(),
        }),
    );
    let backend = TreeBackend::new(parity_listings(), Walk::Items(items), 2);
    let scanned = scan(backend, "/r", None, None);
    assert!(scanned
        .failed
        .iter()
        .any(|(path, detail)| path == "/r/docs/locked" && detail == "Zugriff verweigert"));
    assert_eq!(scanned.rows.len(), 9, "every other row is kept");
    assert!(scanned
        .done
        .as_ref()
        .is_some_and(|progress| progress.errors >= 1));
}

#[test]
fn abgleich_task_whole_tree_links_are_rows_and_hide_what_lies_below_them() {
    let mut items = parity_items();
    // A producer that sends content below a link: it must not be shown.
    items.push(walk_entry("link/inside", false, 9, plain(false)));
    let backend = TreeBackend::new(parity_listings(), Walk::Items(items), 2);
    let scanned = scan(backend, "/r", None, None);
    assert!(scanned
        .rows
        .iter()
        .any(|row| row.path.as_ref() == "/r/link" && row.is_symlink));
    assert!(!scanned
        .rows
        .iter()
        .any(|row| row.path.as_ref() == "/r/link/inside"));
    assert_eq!(scanned.rows.len(), 9);
}

#[test]
fn abgleich_task_whole_tree_depth_limit_reports_the_deepest_folder_only() {
    // Folders down to depth 512 and a file at depth 513.
    let mut items = Vec::new();
    let mut rel = String::new();
    for depth in 1..=513u32 {
        if !rel.is_empty() {
            rel.push('/');
        }
        rel.push('d');
        if depth <= 512 {
            items.push(walk_entry(&rel, true, 0, plain(false)));
        } else {
            items.push(walk_entry(&rel, false, 1, plain(false)));
        }
    }
    let backend = TreeBackend::new(HashMap::new(), Walk::Items(items), 2);
    let scanned = scan(backend, "/r", None, None);
    let deepest = format!("/r{}", "/d".repeat(512));
    let too_deep = format!("/r{}", "/d".repeat(513));
    assert_eq!(
        scanned.rows.len(),
        1 + 512,
        "root plus the folders to depth 512"
    );
    assert!(scanned
        .rows
        .iter()
        .any(|row| row.path.as_ref() == deepest.as_str()));
    assert!(!scanned
        .rows
        .iter()
        .any(|row| row.path.as_ref() == too_deep.as_str()));
    assert!(scanned
        .failed
        .iter()
        .any(|(path, detail)| *path == deepest && detail.contains("Scan-Tiefenlimit")));
    assert!(scanned.truncated);
}

struct TxtOnly;

impl ScanRetention for TxtOnly {
    fn retain(&self, entry: &FileEntry) -> bool {
        !entry.is_dir && entry.name.ends_with(".txt")
    }

    fn descend(&self, _directory: &FileEntry) -> bool {
        true
    }
}

#[test]
fn abgleich_task_filtered_view_keeps_the_folder_walk() {
    let backend = TreeBackend::new(parity_listings(), Walk::Items(parity_items()), 2);
    let retention: RetentionHandle = Arc::new(TxtOnly);
    let scanned = scan(backend.clone(), "/r", None, Some(retention));
    assert_eq!(
        count(&backend.walk_requests),
        0,
        "no whole-tree request with a filter"
    );
    assert!(count(&backend.lists) > 0);
    assert!(scanned.rows.iter().any(|row| row.name.as_ref() == "a.txt"));
}

#[test]
fn abgleich_task_folder_walk_does_not_wait_for_a_slow_sibling() {
    let mut listings = HashMap::new();
    listings.insert(
        "/r".to_string(),
        vec![vmeta("slow", true, 0), vmeta("quick", true, 0)],
    );
    listings.insert("/r/slow".to_string(), Vec::new());
    listings.insert("/r/quick".to_string(), vec![vmeta("q1", true, 0)]);
    listings.insert("/r/quick/q1".to_string(), vec![vmeta("q2", true, 0)]);
    let backend =
        TreeBackend::with_slow_folder(listings, 2, "/r/slow", Duration::from_millis(1000));
    let scanned = scan(backend.clone(), "/r", None, None);
    let order: Vec<String> = backend
        .done_order
        .lock()
        .map(|order| order.clone())
        .unwrap_or_default();
    let slow = order.iter().position(|path| path == "/r/slow");
    let deep = order.iter().position(|path| path == "/r/quick/q1");
    assert!(
        matches!((deep, slow), (Some(deep), Some(slow)) if deep < slow),
        "the sub-folder below the quick sibling waits for the slow one: {order:?}"
    );
    assert_eq!(scanned.rows.len(), 5, "root, slow, quick, q1 and q2");
}
