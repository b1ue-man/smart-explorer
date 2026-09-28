use super::*;
use crate::types::FilterDef;
use std::collections::{HashMap, HashSet};
use std::io;

#[derive(Default)]
struct MockLister {
    dirs: HashMap<String, Vec<Listed>>,
    stats: HashMap<String, Listed>,
    denied: HashSet<String>,
    lists: AtomicUsize,
    statted: AtomicUsize,
}

impl Lister for MockLister {
    fn list(&self, dir: &str) -> io::Result<Vec<Listed>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        if self.denied.contains(dir) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                dir.to_string(),
            ));
        }
        self.dirs
            .get(dir)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, dir.to_string()))
    }

    fn stat(&self, path: &str) -> io::Result<Listed> {
        self.statted.fetch_add(1, Ordering::SeqCst);
        self.stats
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_string()))
    }
}

fn file(name: &str, size: u64) -> Listed {
    Listed {
        name: name.to_string(),
        size,
        ..Listed::default()
    }
}

fn folder(name: &str) -> Listed {
    Listed {
        name: name.to_string(),
        is_dir: true,
        ..Listed::default()
    }
}

/// /src/A/{a.txt, a.md, B/{b.txt}, C/}
fn tree() -> MockLister {
    let mut lister = MockLister::default();
    lister.stats.insert("/src/A".into(), folder("A"));
    lister.stats.insert("/src/A/a.md".into(), file("a.md", 3));
    lister.dirs.insert(
        "/src/A".into(),
        vec![file("a.txt", 5), file("a.md", 3), folder("B"), folder("C")],
    );
    lister
        .dirs
        .insert("/src/A/B".into(), vec![file("b.txt", 7)]);
    lister.dirs.insert("/src/A/C".into(), Vec::new());
    lister
}

fn run(lister: &MockLister, roots: &[WalkRoot], options: &WalkOptions) -> (Vec<WalkEvent>, bool) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let flow = crate::transfer::flow::flow(format!("transfer-engine-task:walk:{nanos}"), None);
    let cancel = AtomicBool::new(false);
    let events = Mutex::new(Vec::new());
    let finished = walk(lister, roots, options, &flow, &cancel, &|event| {
        events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
        true
    });
    (
        events
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        finished,
    )
}

fn root(path: &str, rel: &str) -> WalkRoot {
    WalkRoot {
        path: path.to_string(),
        rel: rel.to_string(),
    }
}

fn position(events: &[WalkEvent], wanted: impl Fn(&WalkEvent) -> bool) -> usize {
    events
        .iter()
        .position(wanted)
        .expect("expected walk event missing")
}

fn is_dir(rel: &'static str) -> impl Fn(&WalkEvent) -> bool {
    move |event: &WalkEvent| matches!(event, WalkEvent::Dir { rel: found, .. } if found == rel)
}

fn is_file(rel: &'static str) -> impl Fn(&WalkEvent) -> bool {
    move |event: &WalkEvent| matches!(event, WalkEvent::File { rel: found, .. } if found == rel)
}

#[test]
fn transfer_engine_task_walk_streams_folders_before_their_contents() {
    let options = WalkOptions {
        folders: true,
        ..WalkOptions::default()
    };
    let (events, finished) = run(&tree(), &[root("/src/A", "A")], &options);
    assert!(finished);
    assert_eq!(position(&events, is_dir("A")), 0);
    assert!(position(&events, is_dir("A/B")) < position(&events, is_file("A/B/b.txt")));
    position(&events, is_dir("A/C"));
    position(&events, is_file("A/a.txt"));
    position(&events, is_file("A/a.md"));
    let files = events
        .iter()
        .filter(|event| matches!(event, WalkEvent::File { .. }))
        .count();
    assert_eq!(files, 3);
}

#[test]
fn transfer_engine_task_walk_filter_keeps_matching_files_only() {
    let mut filter = FilterDef::new();
    filter.extensions = vec!["txt".to_string()];
    let options = WalkOptions {
        filter: crate::transfer::entries::compile_remote_filter(Some((filter, "/src".into()))),
        ..WalkOptions::default()
    };
    let (events, _) = run(&tree(), &[root("/src/A", "A")], &options);
    assert!(events
        .iter()
        .all(|event| !matches!(event, WalkEvent::Dir { .. })));
    position(&events, is_file("A/a.txt"));
    position(&events, is_file("A/B/b.txt"));
    assert!(!events.iter().any(is_file("A/a.md")));
}

#[test]
fn transfer_engine_task_walk_selected_file_passes_any_filter() {
    let mut filter = FilterDef::new();
    filter.extensions = vec!["txt".to_string()];
    let options = WalkOptions {
        filter: crate::transfer::entries::compile_remote_filter(Some((filter, "/src".into()))),
        ..WalkOptions::default()
    };
    let (events, _) = run(&tree(), &[root("/src/A/a.md", "a.md")], &options);
    assert_eq!(events.len(), 1);
    position(&events, is_file("a.md"));
}

#[test]
fn transfer_engine_task_walk_reports_links_and_duplicates_and_continues() {
    let mut lister = MockLister::default();
    lister.stats.insert("/r".into(), folder("r"));
    lister.dirs.insert(
        "/r".into(),
        vec![
            Listed {
                name: "link".into(),
                is_link: true,
                ..Listed::default()
            },
            file("ok.txt", 1),
            file("twice", 1),
            file("twice", 2),
        ],
    );
    let options = WalkOptions {
        folders: true,
        ..WalkOptions::default()
    };
    let (events, finished) = run(&lister, &[root("/r", "r")], &options);
    assert!(finished);
    position(&events, is_file("r/ok.txt"));
    let problems = events
        .iter()
        .filter(|event| matches!(event, WalkEvent::Problem { .. }))
        .count();
    assert_eq!(problems, 2, "link and duplicate name are reported");
}

#[test]
fn transfer_engine_task_walk_flatten_uses_file_names() {
    let options = WalkOptions {
        folders: true,
        flatten: true,
        ..WalkOptions::default()
    };
    let (events, _) = run(&tree(), &[root("/src/A", "A")], &options);
    assert!(events
        .iter()
        .all(|event| !matches!(event, WalkEvent::Dir { .. })));
    position(&events, is_file("b.txt"));
    position(&events, is_file("a.txt"));
}

#[test]
fn transfer_engine_task_walk_stops_when_the_consumer_stops() {
    let options = WalkOptions {
        folders: true,
        ..WalkOptions::default()
    };
    let lister = tree();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let flow = crate::transfer::flow::flow(format!("transfer-engine-task:stop:{nanos}"), None);
    let cancel = AtomicBool::new(false);
    let finished = walk(
        &lister,
        &[root("/src/A", "A")],
        &options,
        &flow,
        &cancel,
        &|_| false,
    );
    assert!(!finished);
}

fn flat_folder(count: usize) -> (MockLister, Vec<WalkRoot>) {
    let mut lister = MockLister::default();
    let mut listed = Vec::new();
    let mut roots = Vec::new();
    for index in 0..count {
        let name = format!("f{index}.bin");
        let path = format!("/many/{name}");
        lister.stats.insert(path.clone(), file(&name, index as u64));
        listed.push(file(&name, index as u64));
        roots.push(root(&path, &name));
    }
    lister.dirs.insert("/many".into(), listed);
    (lister, roots)
}

#[test]
fn transfer_engine_task_walk_many_selected_entries_use_one_listing() {
    let (lister, roots) = flat_folder(500);
    let (events, finished) = run(&lister, &roots, &WalkOptions::default());
    assert!(finished);
    let files = events
        .iter()
        .filter(|event| matches!(event, WalkEvent::File { .. }))
        .count();
    assert_eq!(files, 500);
    assert_eq!(lister.lists.load(Ordering::SeqCst), 1);
    assert_eq!(lister.statted.load(Ordering::SeqCst), 0);
}

#[test]
fn transfer_engine_task_walk_few_selected_entries_are_inspected_directly() {
    let (lister, roots) = flat_folder(2);
    let (events, finished) = run(&lister, &roots, &WalkOptions::default());
    assert!(finished);
    assert_eq!(events.len(), 2);
    assert_eq!(lister.lists.load(Ordering::SeqCst), 0);
    assert_eq!(lister.statted.load(Ordering::SeqCst), 2);
}

#[test]
fn transfer_engine_task_walk_selected_entry_missing_from_listing_is_reported() {
    let (mut lister, mut roots) = flat_folder(40);
    roots.push(root("/many/gone.bin", "gone.bin"));
    lister.stats.remove("/many/gone.bin");
    let (events, finished) = run(&lister, &roots, &WalkOptions::default());
    assert!(finished);
    let problem = position(
        &events,
        |event: &WalkEvent| matches!(event, WalkEvent::Problem { path, .. } if path == "/many/gone.bin"),
    );
    assert!(problem < events.len());
    assert_eq!(lister.statted.load(Ordering::SeqCst), 1);
}

#[test]
fn transfer_engine_task_walk_carries_listed_md5() {
    let mut lister = MockLister::default();
    lister.stats.insert("/d".into(), folder("d"));
    lister.dirs.insert(
        "/d".into(),
        vec![Listed {
            md5: Some("0123456789abcdef0123456789abcdef".into()),
            ..file("x", 4)
        }],
    );
    let (events, _) = run(&lister, &[root("/d", "d")], &WalkOptions::default());
    assert!(events.iter().any(|event| matches!(
        event,
        WalkEvent::File { md5: Some(md5), .. } if md5 == "0123456789abcdef0123456789abcdef"
    )));
}

#[test]
fn transfer_engine_task_walk_backslash_names_only_where_allowed() {
    let mut lister = MockLister::default();
    lister.stats.insert("/d".into(), folder("d"));
    lister.dirs.insert("/d".into(), vec![file("a\\b.txt", 1)]);
    let strict = WalkOptions::default();
    let (events, _) = run(&lister, &[root("/d", "d")], &strict);
    assert!(!events.iter().any(is_file("d/a\\b.txt")));
    position(&events, |event: &WalkEvent| {
        matches!(event, WalkEvent::Problem { .. })
    });
    let local = WalkOptions {
        allow_backslash: true,
        ..WalkOptions::default()
    };
    let (events, _) = run(&lister, &[root("/d", "d")], &local);
    position(&events, is_file("d/a\\b.txt"));
}

#[test]
fn transfer_engine_task_walk_denied_folder_without_elevation_is_a_problem() {
    let mut lister = tree();
    lister.denied.insert("/src/A/B".into());
    let options = WalkOptions {
        folders: true,
        access: Some(Arc::new(crate::transfer::access::AccessGate::new("/src"))),
        ..WalkOptions::default()
    };
    let (events, finished) = run(&lister, &[root("/src/A", "A")], &options);
    assert!(
        finished,
        "a refusal without an elevation path does not stop the walk"
    );
    position(
        &events,
        |event: &WalkEvent| matches!(event, WalkEvent::Problem { path, .. } if path == "/src/A/B"),
    );
    position(&events, is_file("A/a.txt"));
    assert!(!events
        .iter()
        .any(|event| matches!(event, WalkEvent::AccessRefused { .. })));
}

#[test]
fn transfer_engine_task_walk_same_named_files_with_distinct_ids_both_transfer() {
    let with_id = |name: &str, id: &str| Listed {
        id: Some(id.to_string()),
        ..file(name, 1)
    };
    let mut lister = MockLister::default();
    lister.stats.insert("/d".into(), folder("d"));
    lister.dirs.insert(
        "/d".into(),
        vec![
            with_id("a.txt", "one"),
            with_id("a.txt", "two"),
            with_id("a (2).txt", "three"),
            Listed {
                id: Some("four".into()),
                ..folder("sub")
            },
            Listed {
                id: Some("five".into()),
                ..folder("sub")
            },
        ],
    );
    lister.dirs.insert("/d/sub".into(), Vec::new());
    let (events, finished) = run(&lister, &[root("/d", "d")], &WalkOptions::default());
    assert!(finished);
    let renamed = position(
        &events,
        |event: &WalkEvent| matches!(event, WalkEvent::File { rel, id: Some(id), .. } if rel == "d/a (3).txt" && id == "two"),
    );
    assert!(renamed < events.len());
    position(&events, is_file("d/a.txt"));
    position(&events, is_file("d/a (2).txt"));
    position(
        &events,
        |event: &WalkEvent| matches!(event, WalkEvent::Problem { path, .. } if path == "/d/sub"),
    );
}

#[test]
fn transfer_engine_task_walk_same_named_files_without_ids_are_reported() {
    let mut lister = MockLister::default();
    lister.stats.insert("/d".into(), folder("d"));
    lister
        .dirs
        .insert("/d".into(), vec![file("a.txt", 1), file("a.txt", 2)]);
    let (events, _) = run(&lister, &[root("/d", "d")], &WalkOptions::default());
    let files = events
        .iter()
        .filter(|event| matches!(event, WalkEvent::File { .. }))
        .count();
    assert_eq!(files, 1);
    position(&events, |event: &WalkEvent| {
        matches!(event, WalkEvent::Problem { .. })
    });
}
