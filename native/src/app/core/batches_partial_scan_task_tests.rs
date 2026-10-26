use super::*;
use crate::analytics::{ScanIssue, ScanOutcome, ScanStatus, SizeNode};

fn entry(path: &str, size: u64) -> FileEntry {
    let (parent, name) = path.rsplit_once('/').unwrap();
    FileEntry { path: path.into(), parent: parent.into(), name: name.into(), ext: "".into(),
        size, mtime_ms: 0, btime_ms: 0, is_dir: false, is_symlink: false,
        hidden: false, system: false, depth: 2, id: None }
}

#[test]
fn batches_task_partial_entries_keep_omissions_then_replace_only_retry_subtree() {
    let mut entries = vec![entry("root/part/old", 11), entry("root/part/seen", 12),
        entry("root/partial/sibling", 13), entry("root/elsewhere/file", 14)];
    merge_entries(&mut entries, "root/part", vec![entry("root/part/seen", 22),
        entry("root/part/new", 23)], false);
    assert_eq!(entries.len(), 5);
    assert_eq!(entries.iter().find(|e| e.path.as_ref() == "root/part/old").unwrap().size, 11);
    assert_eq!(entries.iter().find(|e| e.path.as_ref() == "root/part/seen").unwrap().size, 22);
    merge_entries(&mut entries, "root/part", vec![entry("root/part/new", 33)], true);
    let facts: std::collections::BTreeMap<_, _> = entries.iter()
        .map(|e| (e.path.as_ref(), e.size)).collect();
    assert_eq!(facts, [("root/elsewhere/file", 14), ("root/part/new", 33),
        ("root/partial/sibling", 13)].into_iter().collect());
    assert_eq!(relative_depth("root/part", "root/partial/sibling"), None);
    assert_eq!(relative_depth("root", "root/part/new"), Some(2));
}

fn node(name: &str, size: u64, children: Vec<SizeNode>) -> SizeNode {
    SizeNode { name: name.into(), size, is_dir: true, children }
}

#[test]
fn batches_task_partial_analytics_restores_denied_data_and_ancestor_totals() {
    let mut previous = node("root", 150, vec![
        node("part", 100, vec![node("denied", 80, vec![]), node("seen", 20, vec![])]),
        node("partial", 50, vec![])]);
    let mut outcome = ScanOutcome::complete(node("part", 30,
        vec![node("denied", 0, vec![]), node("seen", 30, vec![])]));
    outcome.status = ScanStatus::Partial;
    outcome.permission_denied = 1;
    outcome.issues.push(ScanIssue { path: "root/part/denied".into(), detail: "denied".into() });
    outcome.denied_roots.record("root/part/denied");
    outcome.denied_roots.record_size("root/part/denied", 0);
    outcome.denied_roots.record_size("root/part", 30);
    assert!(prepare_partial_analytics(&previous, "root", "root/part", &mut outcome));
    assert_eq!(outcome.denied_roots.roots_with_sizes().unwrap(),
        vec![("root/part/denied".into(), 80)]);
    assert!(outcome.denied_roots.measurements().unwrap().contains(&("root/part".into(), 110)));
    assert!(crate::analytics::merge_retry_tree(&mut previous, &["part"],
        outcome.tree.take().unwrap(), &[100]));
    assert_eq!(previous.size, 160);
    assert_eq!(crate::analytics::retry_node(&previous, &["part", "denied"]).unwrap().size, 80);
    assert_eq!(crate::analytics::retry_node(&previous, &["partial"]).unwrap().size, 50);
    // A second merge of the same measured replacement must not double count.
    assert!(crate::analytics::merge_retry_tree(&mut previous, &["part"],
        node("part", 110, vec![node("denied", 80, vec![]), node("seen", 30, vec![])]), &[110]));
    assert_eq!(previous.size, 160);
}

#[test]
fn batches_task_partial_analytics_refuses_unclassified_errors_and_cancels_on_drop() {
    let previous = node("root", 100, vec![]);
    let mut failed = ScanOutcome::failed("root", "unclassified failure");
    assert!(!prepare_partial_analytics(&previous, "root", "root", &mut failed));
    assert_eq!(previous.size, 100);
    let (_tx, rx) = crossbeam_channel::bounded::<(String, ())>(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker = PartialWorker { generation: 7, root: "root".into(), rx, cancel: cancel.clone() };
    assert!(!cancel.load(Ordering::Acquire));
    drop(worker);
    assert!(cancel.load(Ordering::Acquire));
}
