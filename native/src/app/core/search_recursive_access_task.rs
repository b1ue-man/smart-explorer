//! Focused acceptance of search -> tree presentation -> transfer membership.
use super::prelude::*;
use super::*;
use crate::filter::{parse_extensions, scan_restart_needed, FilterRetention};
use crate::scanner::ScanRetention;

fn entry(path: &str, directory: bool, depth: u32) -> FileEntry {
    let (parent, name) = path
        .trim_end_matches('/')
        .rsplit_once('/')
        .unwrap_or(("", path));
    FileEntry {
        path: Arc::from(path),
        parent: Arc::from(parent),
        name: Arc::from(name),
        ext: Arc::from("deliberately-stale"),
        size: 7,
        mtime_ms: 1,
        btime_ms: 1,
        is_dir: directory,
        is_symlink: false,
        hidden: false,
        system: false,
        depth,
        id: None,
    }
}

fn tree(root: &str) -> Vec<FileEntry> {
    let base = root.trim_end_matches('/');
    vec![
        entry(root, true, 0),
        entry(&format!("{base}/assets"), true, 1),
        entry(&format!("{base}/assets/deep"), true, 2),
        entry(&format!("{base}/assets/deep/model.BLEND"), false, 3),
        entry(&format!("{base}/assets/skip.txt"), false, 2),
        entry(&format!("{base}/empty.blend"), true, 1),
        entry(&format!("{base}/archive.TAR.GZ"), false, 1),
    ]
}

fn result_names(entries: &[FileEntry], rows: &[(usize, u32)]) -> Vec<String> {
    rows.iter()
        .map(|&(index, _)| entries[index].name.to_string())
        .collect()
}

#[test]
fn search_recursive_access_task_literal_suffixes_invalid_patterns_and_scope() {
    let mut filter = FilterDef::new();
    filter.extensions = parse_extensions(" .BLEND;*.heic, tar.gz  .üßx  blend ");
    assert_eq!(filter.extensions, ["blend", "heic", "tar.gz", "üßx"]);
    let compiled = CompiledFilter::compile(&filter);
    for name in ["asset.BLEND", "photo.HEIC", "backup.TAR.GZ", "x.ÜßX"] {
        assert!(
            compiled.matches(&entry(&format!("/r/{name}"), false, 1), "/r"),
            "{name}"
        );
    }
    for name in ["BLEND", "x.blender", "x.gz", ".blend", "x.blend.tmp"] {
        assert!(
            !compiled.matches(&entry(&format!("/r/{name}"), false, 1), "/r"),
            "{name}"
        );
    }
    assert!(!compiled.matches(&entry("/r/folder.blend", true, 1), "/r"));
    let mut narrow = filter.clone();
    filter.extensions = vec!["gz".into()];
    narrow.extensions = vec!["*.TAR.GZ".into()];
    assert!(!scan_restart_needed(Some(&filter), false, &narrow));
    assert!(scan_restart_needed(Some(&narrow), false, &filter));
    filter.extensions.clear();
    filter.text_mode = TextMode::Glob;
    filter.text = "**/*.BLEND".into();
    assert!(CompiledFilter::compile(&filter).matches(&entry("/r/a/x.blend", false, 2), "/r/"));
    for mode in [TextMode::Glob, TextMode::Regex] {
        filter.text_mode = mode;
        filter.text = "[".into();
        let invalid = CompiledFilter::compile(&filter);
        assert!(invalid.error().is_some());
        assert!(!invalid.matches(&entry("/r/x.blend", false, 1), "/r"));
        let mut valid = filter.clone();
        valid.text.clear();
        assert!(scan_restart_needed(Some(&filter), false, &valid));
    }
}

#[test]
fn search_recursive_access_task_roots_orphans_and_hidden_folder_rows() {
    let mut filter = FilterDef::new();
    filter.extensions = vec!["blend".into()];
    for root in ["/", "C:/", "C:/work/", "//server/share/", "/remote/root"] {
        let mut entries = tree(root);
        let rows = recursive_tree::result_rows(&entries, root, &filter, |a, b| {
            entries[a].path.cmp(&entries[b].path)
        });
        assert_eq!(
            result_names(&entries, &rows),
            ["assets", "deep", "model.BLEND"]
        );
        filter.include_dirs = false;
        let retention = FilterRetention::new(filter.clone(), root.into());
        assert!(retention.descend(&entries[1]));
        let rows = recursive_tree::result_rows(&entries, root, &filter, |a, b| a.cmp(&b));
        assert_eq!(result_names(&entries, &rows), ["model.BLEND"]);
        assert_eq!(rows[0].1, 0);
        filter.include_dirs = true;
        entries.retain(|entry| entry.depth == 3);
        let rows = recursive_tree::result_rows(&entries, root, &filter, |a, b| a.cmp(&b));
        assert_eq!(
            result_names(&entries, &rows),
            ["model.BLEND"],
            "orphan at {root}"
        );
    }
    let mut entries = tree("/r");
    entries[1].hidden = true;
    filter.include_hidden = false;
    assert!(recursive_tree::result_rows(&entries, "/r", &filter, |a, b| a.cmp(&b)).is_empty());
    assert!(recursive_tree::relative_path("/root-other/a", "/root").is_none());
}

#[test]
fn search_recursive_access_task_fold_selection_keyboard_and_tab_isolation() {
    let mut app = App::new_for_copy_task();
    app.root_path = "/task".into();
    app.recursive = true;
    app.entries = tree("/task");
    app.filter.extensions = vec!["blend".into()];
    app.recompute_view();
    app.select_all();
    let selected = app.selection.clone();
    app.cursor = Some(app.entries[3].path.clone());
    app.toggle_recursive_folder(1);
    assert_eq!(app.view.len(), 1);
    assert_eq!(app.selection, selected);
    assert_eq!(app.cursor, Some(app.entries[1].path.clone()));
    assert_eq!(app.recursive_transfer_files().len(), 1);
    app.select_all();
    assert_eq!(app.selection, selected);
    app.invert_selection();
    assert!(app.selection.is_empty());
    app.invert_selection();
    assert_eq!(app.selection, selected);
    app.recursive_arrow(true);
    assert_eq!(app.view.len(), 3);
    app.recursive_arrow(true);
    assert_eq!(app.cursor, Some(app.entries[2].path.clone()));
    app.toggle_recursive_folder(1);
    app.tabs.push(TabState::default());
    app.switch_tab(1);
    assert!(!app.recursive);
    assert!(app.tree.collapsed.is_empty());
    app.switch_tab(0);
    assert!(app.recursive);
    assert!(app.tree.collapsed.contains(&app.entries[1].key()));
    assert_eq!(app.selection, selected);
    app.filter.extensions = vec!["gz".into()];
    app.recompute_view();
    assert!(
        app.selection.is_empty(),
        "stale hidden matches must not be copied"
    );
}

fn finish_scan(app: &mut App) {
    let deadline = Instant::now() + std::time::Duration::from_secs(60);
    while app.scan_running {
        app.drain_scan();
        assert!(Instant::now() < deadline, "scan never completed");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.recompute_view();
    assert!(app.error_msg.is_none(), "{:?}", app.error_msg);
}

#[test]
fn search_recursive_access_task_wide_scan_folded_copy_preserves_exact_structure() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("source");
    std::fs::create_dir(&root).unwrap();
    for number in 0..260 {
        let folder = root.join(format!("folder-{number:03}"));
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("asset.BLEND"), b"payload").unwrap();
        std::fs::write(folder.join("exclude.txt"), b"omit").unwrap();
    }
    for number in 0..1100 {
        std::fs::write(root.join(format!("image-{number}.HEIC")), b"payload").unwrap();
    }
    let mut app = App::new_for_copy_task();
    app.recursive = true;
    app.filter.include_dirs = false;
    app.filter.extensions = parse_extensions("*.blend;heic");
    app.start_scan_navigated(root.clone(), false);
    finish_scan(&mut app);
    assert_eq!(app.tree.rows.len(), 1360);
    assert!(app
        .tree
        .rows
        .iter()
        .all(|&(index, _)| !app.entries[index].is_dir));
    app.filter.include_dirs = true;
    app.recompute_view();
    let folder = app
        .entries
        .iter()
        .position(|entry| entry.name.as_ref() == "folder-000")
        .unwrap();
    let file = app
        .entries
        .iter()
        .position(|entry| entry.parent == app.entries[folder].path && !entry.is_dir)
        .unwrap();
    app.selection = HashSet::from([app.entries[folder].key(), app.entries[file].key()]);
    app.toggle_recursive_folder(folder);
    let selected = app.recursive_transfer_files();
    assert_eq!(selected.len(), 1);
    let snapshot = recursive_clipboard::clipboard_snapshot(selected, &app.root_prefix()).unwrap();
    assert_eq!(snapshot[0].rel, "folder-000/asset.BLEND");
    let dest = fixture.path().join("destination");
    let (tx, rx) = unbounded();
    crate::copy::start_copy_pairs(
        snapshot
            .into_iter()
            .map(|file| (file.abs, file.rel))
            .collect(),
        dest.clone(),
        Conflict::Rename,
        tx,
    );
    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
            CopyMsg::Done { progress, errors } => {
                assert!(errors.is_empty(), "{errors:?}");
                assert_eq!(progress.bytes_done, 7);
                break;
            }
            CopyMsg::Progress(_) => {}
        }
    }
    assert_eq!(
        std::fs::read(dest.join("folder-000/asset.BLEND")).unwrap(),
        b"payload"
    );
    assert!(!dest.join("folder-000/exclude.txt").exists());
    assert!(!dest.join("folder-001").exists());
    assert!(root.join("folder-000/asset.BLEND").exists());
}

#[test]
fn search_recursive_access_task_unfiltered_folded_folder_copy_keeps_empty_directories() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    std::fs::create_dir_all(source.join("bundle/empty")).unwrap();
    std::fs::write(source.join("bundle/asset.dat"), b"payload").unwrap();
    let mut app = App::new_for_copy_task();
    app.recursive = true;
    app.filter = FilterDef::new();
    app.start_scan_navigated(source, false);
    finish_scan(&mut app);
    let folder = app
        .entries
        .iter()
        .position(|entry| entry.name.as_ref() == "bundle")
        .unwrap();
    app.selection.insert(app.entries[folder].key());
    app.toggle_recursive_folder(folder);
    app.select_all();
    assert_eq!(
        recursive_clipboard::plain_selection_paths(&app.entries, &app.selection).len(),
        1
    );
    let destination = fixture.path().join("destination");
    app.copy_dest = destination.to_string_lossy().into_owned();
    app.copy_preserve = true;
    app.confirm_copy();
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    while app.copy_rx.is_some() {
        app.drain_copy();
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.copy_errors.is_empty(), "{:?}", app.copy_errors);
    assert!(destination.join("bundle/empty").is_dir());
    assert_eq!(
        std::fs::read(destination.join("bundle/asset.dat")).unwrap(),
        b"payload"
    );
}

#[test]
fn search_recursive_access_task_mid_scan_filter_restart_and_partial_channel() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("match.ZST"), b"x").unwrap();
    let mut app = App::new_for_copy_task();
    app.root_path = fixture.path().to_string_lossy().replace('\\', "/");
    app.recursive = true;
    app.filter.extensions = vec!["zst".into()];
    app.scan_retention = None;
    app.note_scan_finished(true);
    assert!(
        app.scan_running,
        "narrowed truncated scan must restart once"
    );
    finish_scan(&mut app);
    assert_eq!(result_names(&app.entries, &app.view), ["match.ZST"]);
    app.note_scan_finished(true);
    assert!(!app.scan_running, "unchanged filter must not loop");
    let (tx, rx) = unbounded();
    app.scan_rx = Some(rx);
    app.scan_running = true;
    tx.send(ScanMessage::FailedPaths(vec![(
        "denied".into(),
        "permission denied".into(),
    )]))
    .unwrap();
    let mut progress = empty_progress();
    progress.errors = 1;
    progress.permission_denied = 1;
    tx.send(ScanMessage::Done(progress)).unwrap();
    app.drain_scan();
    assert_eq!(app.view.len(), 1);
    assert_eq!(app.progress.permission_denied, 1);
    assert_eq!(app.failed_paths.len(), 1);
}

#[test]
fn search_recursive_access_task_long_clipboard_snapshot_and_remote_materialization() {
    let fixture = tempfile::tempdir().unwrap();
    let mut source = fixture.path().to_path_buf();
    let names: Vec<_> = (0..5)
        .map(|i| format!("folder-{i}-{}", "x".repeat(48)))
        .collect();
    for part in &names {
        source.push(part);
    }
    std::fs::create_dir_all(&source).unwrap();
    source.push("selected.blend");
    std::fs::write(&source, b"payload").unwrap();
    let path = source.to_string_lossy().replace('\\', "/");
    let root = fixture.path().to_string_lossy().replace('\\', "/");
    let snapshot =
        recursive_clipboard::clipboard_snapshot(vec![entry(&path, false, 6)], &root).unwrap();
    assert!(snapshot[0].rel.encode_utf16().count() >= 260);
    let backend = crate::vfs::LocalBackend::new(&root);
    let copied = download_clipboard_snapshot(&backend, snapshot).unwrap();
    assert_eq!(copied.len(), 1);
    let mut target = PathBuf::from(&copied[0]);
    for part in &names[1..] {
        target.push(part);
    }
    target.push("selected.blend");
    assert_eq!(std::fs::read(target).unwrap(), b"payload");
    cleanup_temp_copy(Path::new(&copied[0]));
    assert!(recursive_clipboard::clipboard_snapshot(
        vec![entry("/elsewhere/secret", false, 1)],
        "/allowed"
    )
    .is_err());
}
