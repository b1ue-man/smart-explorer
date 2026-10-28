//! M19: a finished transfer's roots are merged into the view instead of a
//! full rescan; the merged listing equals what a full rescan would list.
use super::super::transfer_route::{paste_job, TransferPlace, TransferSelection};
use super::*;
use std::sync::atomic::AtomicBool;

fn roots_of(plan: &[ChangedRoot]) -> Vec<(&str, &str, &str, u32)> {
    let mut found = Vec::new();
    for root in plan {
        found.push((
            root.path.as_str(),
            root.parent.as_str(),
            root.name.as_str(),
            root.depth,
        ));
    }
    found
}

#[test]
fn speed_task_view_refresh_plans_outermost_roots_within_the_view_depth() {
    let paths: Vec<String> = ["/v/a", "/v/a/b", "/v/sub/deep", "/w/x", "/v/c"]
        .iter()
        .map(|path| path.to_string())
        .collect();
    let flat = plan_roots("/v", "/v", Some(1), &paths).expect("only rows change");
    assert_eq!(
        roots_of(&flat),
        [("/v/a", "/v", "a", 1), ("/v/c", "/v", "c", 1)]
    );
    let deep = plan_roots("/v", "/v", None, &paths).expect("only subtrees change");
    assert_eq!(
        roots_of(&deep),
        [
            ("/v/a", "/v", "a", 1),
            ("/v/c", "/v", "c", 1),
            ("/v/sub/deep", "/v/sub", "deep", 2),
        ],
        "/v/a/b is covered by /v/a, /w/x is outside"
    );
    assert_eq!(plan_roots("/v", "/v", None, &["/v".to_string()]), None);
    assert_eq!(plan_roots("/v/a", "/v/a", None, &["/v".to_string()]), None);
    let top = plan_roots("/", "/", Some(1), &["/a".to_string()]).expect("root view");
    assert_eq!(roots_of(&top), [("/a", "/", "a", 1)]);
}

#[test]
fn speed_task_view_refresh_takes_resolved_roots_and_moved_sources() {
    let selection = TransferSelection::roots(
        TransferPlace::local(),
        vec!["/src/a.txt".into(), "/src/dir".into()],
        Some("/src".into()),
    );
    let resolved = [
        ResolvedRoot {
            source: "/src/a.txt".into(),
            rel: "a (2).txt".into(),
        },
        ResolvedRoot {
            source: "/src/dir".into(),
            rel: "dir/inner".into(),
        },
    ];
    let copy = paste_job(&selection, &TransferPlace::local(), "/v", CopyMode::Copy).unwrap();
    assert_eq!(
        changed_paths(&copy, &resolved),
        Some(vec!["/v/a (2).txt".to_string(), "/v/dir".to_string()])
    );
    let moved = paste_job(&selection, &TransferPlace::local(), "/v", CopyMode::Move).unwrap();
    assert_eq!(
        changed_paths(&moved, &resolved),
        Some(vec![
            "/v/a (2).txt".to_string(),
            "/v/dir".to_string(),
            "/src/a.txt".to_string(),
            "/src/dir".to_string(),
        ])
    );
    assert_eq!(changed_paths(&copy, &[]), None, "nothing resolved");
    let into_target = [ResolvedRoot {
        source: "/src/dir".into(),
        rel: String::new(),
    }];
    assert_eq!(
        changed_paths(&copy, &into_target),
        None,
        "the target itself"
    );
}

/// A full scan of `view`, as the view lists it.
fn full_scan(view: &Path, max_depth: Option<u32>) -> Vec<FileEntry> {
    let text = view.to_string_lossy().replace('\\', "/");
    let (entries, complete) =
        scan(&text, 0, max_depth, None, &AtomicBool::new(false)).expect("scan ran");
    assert!(complete, "the fixture scans without errors");
    entries
}

fn facts(entries: &[FileEntry]) -> Vec<(String, u32, u64, bool)> {
    let mut facts: Vec<_> = entries
        .iter()
        .map(|entry| {
            (
                entry.path.to_string(),
                entry.depth,
                entry.size,
                entry.is_dir,
            )
        })
        .collect();
    facts.sort();
    facts
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn speed_task_view_refresh_merge_equals_a_full_rescan() {
    for max_depth in [Some(1), None] {
        let fixture = tempfile::tempdir().unwrap();
        let view = fixture.path().join("v");
        write(&view.join("alt.txt"), b"old");
        write(&view.join("keep.txt"), b"keep");
        write(&view.join("ordner/a.txt"), b"a");
        let mut entries = full_scan(&view, max_depth);
        let prefix = entries
            .iter()
            .find(|entry| entry.depth == 0)
            .map(|entry| entry.path.to_string())
            .expect("view root row");
        // The transfer: two new roots, one merged folder, one moved source.
        write(&view.join("neu.txt"), b"neu!");
        write(&view.join("paket/x/y.txt"), b"tief");
        write(&view.join("ordner/b.txt"), b"bb");
        std::fs::remove_file(view.join("alt.txt")).unwrap();
        let paths: Vec<String> = ["neu.txt", "paket", "ordner", "alt.txt"]
            .iter()
            .map(|name| format!("{prefix}/{name}"))
            .collect();
        let roots = plan_roots(&prefix, &prefix, max_depth, &paths).expect("rows change");
        assert_eq!(roots.len(), 4);
        let listings = read_roots(&roots, max_depth, None, &AtomicBool::new(false));
        let before = entries.len() as i128;
        let (count, _) = merge_listings(&mut entries, listings);
        let rescanned = full_scan(&view, max_depth);
        assert_eq!(
            facts(&entries),
            facts(&rescanned),
            "depth limit {max_depth:?}"
        );
        assert_eq!(before + count, rescanned.len() as i128);
    }
}

#[test]
fn speed_task_view_refresh_merge_handles_twice_listed_and_partial_roots() {
    let row = |path: &str, size: u64| {
        let (parent, name) = path.rsplit_once('/').unwrap();
        FileEntry {
            path: path.into(),
            parent: parent.into(),
            name: name.into(),
            ext: "".into(),
            size,
            mtime_ms: 0,
            btime_ms: 0,
            is_dir: false,
            is_symlink: false,
            hidden: false,
            system: false,
            depth: path.matches('/').count() as u32 - 1,
            id: None,
        }
    };
    let mut entries = vec![row("/v/a", 1), row("/v/d/old", 2), row("/v/e/kept", 3)];
    let listing = |root: &str, rows: Vec<FileEntry>, complete: bool| RootListing {
        root: root.into(),
        entries: rows,
        complete,
    };
    let (count, bytes) = merge_listings(
        &mut entries,
        vec![
            // Two transfers into the same folder report the same root.
            listing("/v/b", vec![row("/v/b", 5)], true),
            listing("/v/b", vec![row("/v/b", 5)], true),
            // A folder read completely replaces its old rows.
            listing("/v/d", vec![row("/v/d/new", 7)], true),
            // A partly read folder only adds and updates.
            listing(
                "/v/e",
                vec![row("/v/e/kept", 4), row("/v/e/more", 6)],
                false,
            ),
        ],
    );
    assert_eq!(
        facts(&entries),
        facts(&[
            row("/v/a", 1),
            row("/v/b", 5),
            row("/v/d/new", 7),
            row("/v/e/kept", 4),
            row("/v/e/more", 6),
        ])
    );
    assert_eq!((count, bytes), (2, 5 + 7 - 2 + 1 + 6));
}

#[test]
fn speed_task_view_insert_keeps_sort_order_and_selection() {
    let mut app = App::new_for_copy_task();
    let fixture = tempfile::tempdir().unwrap();
    let view = fixture.path().join("v");
    write(&view.join("b.txt"), b"b");
    write(&view.join("d.txt"), b"d");
    app.entries = full_scan(&view, Some(1));
    let prefix = app.root_prefix();
    app.root_path = prefix.clone();
    app.recursive = false;
    app.sort_key = SortKey::Name;
    app.sort_dir = SortDir::Asc;
    app.dirs_first = false;
    app.recompute_view();
    let selected: Arc<str> = Arc::from(format!("{prefix}/d.txt"));
    app.selection.insert(selected.clone());
    app.cursor = Some(selected.clone());
    app.last_anchor = Some(selected.clone());

    write(&view.join("a.txt"), b"a");
    write(&view.join("c.txt"), b"c");
    let paths = vec![format!("{prefix}/a.txt"), format!("{prefix}/c.txt")];
    let roots = plan_roots(&prefix, &prefix, Some(1), &paths).expect("rows change");
    let listings = read_roots(&roots, Some(1), None, &AtomicBool::new(false));
    app.apply_root_listings(listings);

    let names: Vec<&str> = app
        .view
        .iter()
        .map(|&(index, _)| app.entries[index].name.as_ref())
        .collect();
    assert_eq!(names, ["a.txt", "b.txt", "c.txt", "d.txt"]);
    assert_eq!(app.selection, HashSet::from([selected.clone()]));
    assert_eq!(app.cursor, Some(selected.clone()));
    assert_eq!(app.last_anchor, Some(selected));
    assert!(!app.scan_running, "no rescan was started");
}
