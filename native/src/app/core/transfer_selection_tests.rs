//! What a copy takes from the view: the filtered snapshot with what the tree
//! in memory knows, the outermost selected entries, and paths from the OS.
use super::super::recursive_clipboard::outer_selection_roots;
use super::*;
use crate::transfer::JobItems;

fn entry(path: &str, is_dir: bool, size: u64, id: Option<&str>) -> FileEntry {
    let (parent, name) = path.rsplit_once('/').expect("absolute test path");
    FileEntry {
        path: Arc::from(path),
        parent: Arc::from(parent),
        name: Arc::from(name),
        ext: Arc::from(""),
        size,
        mtime_ms: 11,
        btime_ms: 0,
        is_dir,
        is_symlink: false,
        hidden: false,
        system: false,
        depth: path.matches('/').count() as u32,
        id: id.map(Arc::from),
    }
}

#[test]
fn transfer_engine_task_snapshot_pairs_carry_what_the_view_knows() {
    let files = vec![
        entry("/root/a/x.txt", false, 5, None),
        entry("/root/b.bin", false, 7, Some("drive-id-7")),
    ];
    let pairs = snapshot_pairs(&files, "/root").expect("pairs");
    assert_eq!(
        pairs[0],
        PairItem {
            source: "/root/a/x.txt".to_string(),
            rel: "a/x.txt".to_string(),
            size: Some(5),
            mtime_ms: 11,
            id: None,
        }
    );
    assert_eq!(pairs[1].rel, "b.bin");
    assert_eq!(pairs[1].size, Some(7));
    assert_eq!(pairs[1].id.as_deref(), Some("drive-id-7"));
    // The view root's own trailing slash does not change the destinations.
    assert_eq!(
        snapshot_pairs(&files, "/root/").expect("pairs")[0].rel,
        "a/x.txt"
    );
    assert!(snapshot_pairs(&[entry("/elsewhere/y", false, 1, None)], "/root").is_err());
}

#[test]
fn transfer_engine_task_outer_roots_leave_out_contents_of_selected_folders() {
    let entries = vec![
        entry("/r/dir", true, 0, None),
        entry("/r/dir/inner.txt", false, 1, None),
        entry("/r/file.txt", false, 1, None),
        // Two Drive files of one name are one path for whole-entry copies.
        entry("/r/same.doc", false, 1, Some("one")),
        entry("/r/same.doc", false, 1, Some("two")),
        entry("/r/unselected.txt", false, 1, None),
    ];
    let selected: HashSet<Arc<str>> = entries[..5].iter().map(FileEntry::key).collect();
    assert_eq!(
        outer_selection_roots(&entries, &selected),
        vec!["/r/dir", "/r/file.txt", "/r/same.doc"]
    );
    assert!(outer_selection_roots(&entries, &HashSet::new()).is_empty());
}

#[test]
fn transfer_engine_task_os_paths_keep_the_first_folder_as_base() {
    let native = |path: &str| path.replace('/', std::path::MAIN_SEPARATOR_STR);
    let selection = os_paths_selection(vec![native("/data/a.txt"), native("/data/sub/b.txt")])
        .expect("selection");
    assert!(selection.source.is_local());
    assert!(selection.filter.is_none());
    assert_eq!(
        selection.items,
        JobItems::Roots {
            paths: vec!["/data/a.txt".to_string(), "/data/sub/b.txt".to_string()],
            base: Some("/data".to_string()),
        }
    );
    assert!(os_paths_selection(Vec::new()).is_none());
}
