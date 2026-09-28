use super::*;
use crate::types::FilterDef;
use std::collections::HashMap;
use std::io;

#[derive(Default)]
struct MockLister {
    dirs: HashMap<String, Vec<Listed>>,
    stats: HashMap<String, Listed>,
}

impl Lister for MockLister {
    fn list(&self, dir: &str) -> io::Result<Vec<Listed>> {
        self.dirs
            .get(dir)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, dir.to_string()))
    }

    fn stat(&self, path: &str) -> io::Result<Listed> {
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
        filter: None,
        folders: true,
        flatten: false,
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
        folders: false,
        flatten: false,
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
        folders: false,
        flatten: false,
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
        filter: None,
        folders: true,
        flatten: false,
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
        filter: None,
        folders: true,
        flatten: true,
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
        filter: None,
        folders: true,
        flatten: false,
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
