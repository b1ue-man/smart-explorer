//! `scan.view` answers from a snapshot taken under the lock: the scan consumer is
//! not kept waiting for a tree build, a batch that arrives during a view does not
//! change the entries the view answers from, and the answer keys keep their
//! meaning. Test names start with `abgleich_task_`.
use super::*;

fn session() -> ScanSession {
    ScanSession {
        base: Loc::parse("/storage/emulated/0/tree").expect("local location"),
        filter: pass_all(false),
        data: Mutex::new(ScanData::default()),
        finished: AtomicBool::new(true),
    }
}

/// Folder `number` of the tree, at depth 1.
fn folder(number: usize) -> FileEntry {
    let path = format!("/storage/emulated/0/tree/d{number}");
    FileEntry {
        path: Arc::from(path.as_str()),
        parent: Arc::from("/storage/emulated/0/tree"),
        name: Arc::from(format!("d{number}").as_str()),
        ext: Arc::from(""),
        size: 0,
        mtime_ms: 1,
        btime_ms: 0,
        is_dir: true,
        is_symlink: false,
        hidden: false,
        system: false,
        depth: 1,
        id: None,
    }
}

/// File `number`, in folder `number % 7`, at depth 2.
fn file(number: usize) -> FileEntry {
    let parent = format!("/storage/emulated/0/tree/d{}", number % 7);
    FileEntry {
        path: Arc::from(format!("{parent}/f{number}.txt").as_str()),
        parent: Arc::from(parent.as_str()),
        name: Arc::from(format!("f{number}.txt").as_str()),
        ext: Arc::from("txt"),
        size: number as u64,
        mtime_ms: 1_000 + number as i64,
        btime_ms: 0,
        is_dir: false,
        is_symlink: false,
        hidden: false,
        system: false,
        depth: 2,
        id: None,
    }
}

fn fill(session: &ScanSession, batch: Vec<FileEntry>) {
    push_entries(&mut lock(&session.data), batch);
}

#[test]
fn abgleich_task_scan_view_snapshot_leaves_the_lock_free_for_the_consumer() {
    let session = session();
    fill(
        &session,
        (0..7).map(folder).chain((0..30).map(file)).collect(),
    );
    let snapshot = session.snapshot(SortSpec::default(), "key", None);
    assert!(
        session.data.try_lock().is_ok(),
        "the scan consumer can take the lock while a view holds its snapshot"
    );
    assert_eq!(snapshot.entries.len(), 37, "the snapshot holds every entry");
}

#[test]
fn abgleich_task_scan_view_batch_after_a_snapshot_leaves_the_snapshot_as_it_was() {
    let session = session();
    fill(&session, (0..10).map(file).collect());
    let before = session.snapshot(SortSpec::default(), "key", None);
    fill(&session, (10..20).map(file).collect());
    assert_eq!(
        before.entries.len(),
        10,
        "the earlier snapshot keeps its entries"
    );
    let after = session.snapshot(SortSpec::default(), "key", None);
    assert_eq!(after.entries.len(), 20, "the next snapshot has the batch");
    assert_eq!(
        after.revision,
        before.revision + 1,
        "a batch is one revision"
    );
}

#[test]
fn abgleich_task_scan_view_answers_unchanged_for_the_same_revision_and_window() {
    let session = session();
    fill(
        &session,
        (0..7).map(folder).chain((0..30).map(file)).collect(),
    );
    let sort = SortSpec::default();
    let collapsed = HashSet::new();
    let rows = tree_rows(
        &session.snapshot(sort, "key", None).entries,
        &session.base.path,
        &session.filter,
        sort,
    )
    .rows
    .len();
    let first = view_answer(&session, sort, &collapsed, 0, 10, None);
    assert_eq!(
        first["unchanged"],
        json!(false),
        "the first answer carries rows"
    );
    assert_eq!(
        first["entries"].as_array().map(Vec::len),
        Some(rows.min(10)),
        "the window holds the first rows of the tree"
    );
    assert_eq!(
        first["visibleTotal"],
        json!(rows),
        "the total is the row count of the tree"
    );
    let revision = first["revision"].as_i64().expect("a revision");
    let again = view_answer(&session, sort, &collapsed, 0, 10, Some(revision));
    assert_eq!(
        again["unchanged"],
        json!(true),
        "the same revision and window are unchanged"
    );
    assert_eq!(
        again["entries"].as_array().map(Vec::len),
        Some(0),
        "an unchanged answer carries no rows"
    );
    assert_eq!(again["visibleTotal"], json!(rows));
    fill(&session, vec![file(30)]);
    let moved = view_answer(&session, sort, &collapsed, 0, 10, Some(revision));
    assert_eq!(
        moved["unchanged"],
        json!(false),
        "a new batch changes the answer"
    );
    assert_eq!(moved["revision"].as_i64(), Some(revision + 1));
}

#[test]
fn abgleich_task_scan_view_keeps_the_tree_for_its_revision_only() {
    let session = session();
    fill(
        &session,
        (0..7).map(folder).chain((0..30).map(file)).collect(),
    );
    let sort = SortSpec::default();
    let collapsed = HashSet::new();
    let _answer = view_answer(&session, sort, &collapsed, 0, 10, None);
    assert!(
        session.snapshot(sort, "key", None).tree.is_some(),
        "the answer kept its tree for this revision"
    );
    fill(&session, vec![file(30)]);
    assert!(
        session.snapshot(sort, "key", None).tree.is_none(),
        "a new revision has no tree yet"
    );
}
