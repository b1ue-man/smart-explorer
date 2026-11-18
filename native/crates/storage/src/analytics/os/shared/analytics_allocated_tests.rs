//! Occupied bytes of the size scan: one count per hard-link group, exact
//! sums through retained and aggregated nodes, unknown where a file did not
//! say.
use super::*;

fn file(name: &str, size: u64, allocated: Option<u64>, link: Option<(u64, u64)>) -> LocalEntry {
    LocalEntry {
        name: name.into(),
        kind: EntryKind::File,
        size,
        allocated,
        hardlink: link,
        ..Default::default()
    }
}

fn walk(entries: Vec<LocalEntry>, nodes: u64) -> (ScanOutcome, Progress) {
    let fixture = tempfile::tempdir().expect("temp dir");
    let progress = Progress::default();
    let diagnostics = Diagnostics::default();
    let budget = AnalyticsBudget::with_limits(nodes, u64::MAX, 16);
    let traversal = Traversal {
        progress: &progress,
        diagnostics: &diagnostics,
        budget: &budget,
        parallel: false,
        guard: None,
        device: None,
    };
    let tree = scan_entries(
        &traversal,
        fixture.path(),
        "root".into(),
        Ok(entries.into_iter().map(Ok)),
        0,
        true,
    );
    (diagnostics.finish(tree, false), progress)
}

#[test]
fn belegt_task_walk_counts_links_once_and_keeps_aggregates_exact() {
    let (outcome, progress) = walk(
        vec![
            file("a", 100, Some(4096), Some((1, 7))),
            file("b", 100, Some(4096), Some((1, 7))),
            // A cloud placeholder: 5 GB of size, nothing on the volume.
            file("placeholder", 5_000_000_000, Some(0), None),
            file("d", 10, Some(4096), None),
        ],
        2,
    );
    assert!(outcome.allocated_known);
    assert_eq!(outcome.allocated_bytes(), Some(8192));
    let tree = outcome.tree.expect("tree");
    assert_eq!(tree.size, 5_000_000_210);
    assert_eq!(tree.allocated, 8192);
    // Two retained nodes, the rest in one aggregate; both sums stay exact.
    assert_eq!(tree.children.len(), 3);
    let sum = |pick: fn(&SizeNode) -> u64| tree.children.iter().map(pick).sum::<u64>();
    assert_eq!(sum(|node| node.size), tree.size);
    assert_eq!(sum(|node| node.allocated), tree.allocated);
    let linked: Vec<u64> = tree.children[..2].iter().map(|n| n.allocated).collect();
    assert_eq!(linked, [4096, 0], "the second link occupies nothing more");
    assert_eq!(tree.children[2].allocated, 4096);
    assert_eq!(progress.bytes.load(Ordering::Relaxed), 5_000_000_210);
    assert_eq!(progress.allocated.load(Ordering::Relaxed), 8192);
}

#[test]
fn belegt_task_walk_without_allocation_counts_size_as_unknown() {
    let (outcome, progress) = walk(
        vec![
            file("known", 10, Some(4096), None),
            file("unknown", 300, None, None),
        ],
        64,
    );
    assert!(!outcome.allocated_known);
    let tree = outcome.tree.expect("tree");
    assert_eq!(tree.allocated, 4096 + 300);
    assert_eq!(progress.allocated.load(Ordering::Relaxed), 4096 + 300);
    // A walk without files has nothing unknown.
    let (empty, _) = walk(Vec::new(), 64);
    assert!(empty.allocated_known);
}

/// The real platform listing: a sparse file occupies less than its size and
/// a file with two links is counted once in the occupied total.
#[cfg(unix)]
#[test]
fn belegt_task_local_scan_counts_hardlink_once() {
    let fixture = tempfile::tempdir().expect("temp dir");
    let base = fixture.path();
    std::fs::write(base.join("x.bin"), vec![1u8; 10_000]).expect("write");
    std::fs::create_dir(base.join("sub")).expect("dir");
    std::fs::hard_link(base.join("x.bin"), base.join("sub/y.bin")).expect("link");
    let sparse = std::fs::File::create(base.join("sparse.img")).expect("create");
    sparse.set_len(64 * 1024 * 1024).expect("extend");
    drop(sparse);
    let progress = Progress::default();
    let outcome = scan(base, &progress);
    assert_eq!(outcome.status, ScanStatus::Complete);
    assert!(outcome.allocated_known);
    let tree = outcome.tree.expect("tree");
    assert_eq!(tree.size, 2 * 10_000 + 64 * 1024 * 1024);
    let node = |path: &[&str]| {
        let mut node = &tree;
        for name in path {
            node = node
                .children
                .iter()
                .find(|child| &*child.name == *name)
                .expect("node");
        }
        node.allocated
    };
    let (x, y) = (node(&["x.bin"]), node(&["sub", "y.bin"]));
    assert!(x.max(y) >= 10_000);
    assert_eq!(x.min(y), 0, "one link carries the file, the other nothing");
    assert!(node(&["sparse.img"]) < 64 * 1024 * 1024);
    assert_eq!(
        tree.allocated,
        x + y + node(&["sparse.img"]),
        "the root sums the occupied bytes exactly"
    );
    assert_eq!(progress.allocated.load(Ordering::Relaxed), tree.allocated);
}

/// An agent tree with occupancy is known; without it (old agent, `Tree`
/// frame) every node occupies its size and the folder sums stay exact.
#[test]
fn belegt_task_agent_tree_known_only_with_every_node() {
    use crate::agent_proto::WireNode;
    let file = |name: &str, size, allocated| WireNode {
        name: name.into(),
        size,
        allocated,
        is_dir: false,
        children: Vec::new(),
    };
    let tree = |a, b, root| WireNode {
        name: "root".into(),
        size: 300,
        allocated: root,
        is_dir: true,
        children: vec![file("a", 100, a), file("b", 200, b)],
    };
    let progress = Progress::default();
    let known = crate::analytics::finish_legacy_tree(
        tree(Some(4096), Some(0), Some(4096)),
        &progress,
        0,
        0,
    );
    assert!(known.allocated_known);
    assert_eq!(known.allocated_bytes(), Some(4096));
    assert_eq!(progress.allocated.load(Ordering::Relaxed), 4096);

    let mixed = crate::analytics::finish_legacy_tree(
        tree(Some(4096), None, None),
        &Progress::default(),
        0,
        0,
    );
    assert!(!mixed.allocated_known);
    let mixed = mixed.tree.expect("tree");
    assert_eq!(mixed.allocated, 4096 + 200);
    let sum: u64 = mixed.children.iter().map(|child| child.allocated).sum();
    assert_eq!(sum, mixed.allocated);
}

fn walk_with(
    entries: Vec<LocalEntry>,
    budget: AnalyticsBudget,
    device: Option<u64>,
) -> ScanOutcome {
    let fixture = tempfile::tempdir().expect("temp dir");
    let progress = Progress::default();
    let diagnostics = Diagnostics::default();
    let traversal = Traversal {
        progress: &progress,
        diagnostics: &diagnostics,
        budget: &budget,
        parallel: false,
        guard: None,
        device,
    };
    let tree = scan_entries(
        &traversal,
        fixture.path(),
        "root".into(),
        Ok(entries.into_iter().map(Ok)),
        0,
        true,
    );
    diagnostics.finish(tree, false)
}

/// A folder on another file system is a visible note and not counted; an
/// unknown device never excludes.
#[test]
fn analyse_task_walk_stays_on_the_root_file_system() {
    let folder = |name: &str, device| LocalEntry {
        name: name.into(),
        kind: EntryKind::Directory,
        is_dir: true,
        device,
        ..Default::default()
    };
    let outcome = walk_with(
        vec![
            folder("mounted", Some(2)),
            folder("same", Some(1)),
            folder("unknown", None),
        ],
        AnalyticsBudget::default(),
        Some(1),
    );
    // The listed folders do not exist here; only which ones were walked
    // counts.
    let names: Vec<&str> = outcome
        .tree
        .as_ref()
        .expect("tree")
        .children
        .iter()
        .map(|child| &*child.name)
        .collect();
    assert_eq!(names, ["same", "unknown"]);
    assert!(outcome.notes.iter().any(|note| note
        .starts_with("anderes Dateisystem, nicht gezählt:")
        && note.ends_with("mounted")));
}

/// The hard-link set has a limit: beyond it files count (possibly twice)
/// and the analysis says so once.
#[test]
fn analyse_task_hardlink_set_limit_is_noted() {
    let outcome = walk_with(
        vec![
            file("a", 10, Some(4096), Some((7, 1))),
            file("b", 10, Some(4096), Some((7, 1))),
            file("c", 10, Some(4096), Some((7, 2))),
            file("d", 10, Some(4096), Some((7, 3))),
        ],
        AnalyticsBudget::default().with_hardlink_limit(1),
        None,
    );
    assert_eq!(outcome.tree.expect("tree").allocated, 3 * 4096);
    let noted = outcome
        .notes
        .iter()
        .filter(|note| note.contains("möglicherweise mehrfach gezählt"))
        .count();
    assert_eq!(noted, 1);
}

/// Folder names reach the progress at most every 50 ms per thread.
#[test]
fn analyse_task_directory_names_are_throttled() {
    std::thread::spawn(|| {
        assert!(Progress::directory_due());
        assert!(!Progress::directory_due());
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(Progress::directory_due());
    })
    .join()
    .expect("thread");
}
