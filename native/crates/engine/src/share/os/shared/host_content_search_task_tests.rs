//! `content_search_v1` on the host (plan 2026-11-27 section B): the search
//! runs over a local export through held handles, finds the hits, reports
//! nothing outside the export's rights (Share-private names, folders the
//! export does not hold, links in the export), holds `max_hits` over the
//! whole search and ends when its stream ends.
use super::*;
use crate::share::{
    export_config::ExportAccess,
    fs::{ShareExportConfig, SharedRoot},
    fs_host_policy_task_tests::text,
    fs_response::FsContentQuery,
};
use std::collections::BTreeMap;

const NEEDLE: &[u8] = b"one needle here\n";

/// An export "Docs" with hits in two files, the same text in Share-private
/// places inside it and in a folder next to it that is not exported, and
/// (Unix) links from the export to that folder.
fn fixture() -> (tempfile::TempDir, FsAccess) {
    let directory = tempfile::tempdir().unwrap();
    let base = directory.path().canonicalize().unwrap();
    let root = base.join("Docs");
    let outside = base.join("outside");
    for folder in [root.join("sub"), root.join(".SE-Versions"), outside.clone()] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(root.join("a.txt"), NEEDLE).unwrap();
    std::fs::write(
        root.join("sub").join("b.md"),
        b"needle\nplain\nneedle again\n",
    )
    .unwrap();
    std::fs::write(root.join("plain.txt"), b"nothing to see\n").unwrap();
    std::fs::write(root.join(".SE-Versions").join("old.txt"), NEEDLE).unwrap();
    std::fs::write(root.join(".held.se-recycle-FEDCBA9876543210"), NEEDLE).unwrap();
    std::fs::write(outside.join("secret.txt"), NEEDLE).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("linked.txt")).unwrap();
    }
    let access = FsAccess::dynamic(ShareExportConfig {
        roots: vec![SharedRoot::new("Docs", text(&root)).with_access(ExportAccess::ReadOnly)],
        ..Default::default()
    });
    (directory, access)
}

fn request(path: &str, query: &ContentQuery) -> FsContentSearch {
    FsContentSearch {
        path: path.into(),
        query: FsContentQuery::from(query),
    }
}

/// The events and the closing totals of one finished search.
fn search(
    access: &FsAccess,
    path: &str,
    query: &ContentQuery,
) -> (Vec<FsContentEvent>, FsContentSummary) {
    let (tx, mut rx) = mpsc::channel(1024);
    run(
        request(path, query),
        access.clone(),
        tx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(Totals::default()),
    )
    .expect("the search runs");
    let mut events = Vec::new();
    let mut done = None;
    while let Ok(item) = rx.try_recv() {
        match item.expect("no failure in the stream") {
            FsResponse::ContentSearch {
                message: FsContentMessage::Events { events: portion },
            } => events.extend(portion),
            FsResponse::ContentSearch {
                message: FsContentMessage::Done { summary },
            } => done = Some(summary),
            _ => {}
        }
    }
    (events, done.expect("closing totals"))
}

/// Hit lines per file.
fn hits(events: &[FsContentEvent]) -> BTreeMap<String, u64> {
    let mut files = BTreeMap::new();
    for event in events {
        if let FsContentEvent::Hits { rel, lines, .. } = event {
            *files.entry(rel.clone()).or_default() +=
                lines.iter().filter(|line| !line.context).count() as u64;
        }
    }
    files
}

#[test]
fn content_search_task_host_finds_hits_only_within_the_export_rights() {
    let (_directory, access) = fixture();
    let mut query = ContentQuery::text("needle");
    // Hidden folders are entered: the private names stay out by the rights.
    query.hidden = true;
    for (path, prefix) in [("/Docs", ""), ("/", "Docs/")] {
        let (events, summary) = search(&access, path, &query);
        let expected: BTreeMap<String, u64> = [
            (format!("{prefix}a.txt"), 1),
            (format!("{prefix}sub/b.md"), 2),
        ]
        .into_iter()
        .collect();
        assert_eq!(hits(&events), expected, "{path}: {events:?}");
        for event in &events {
            let rel = event.rel().to_ascii_lowercase();
            assert!(
                !rel.contains(".se-versions")
                    && !rel.contains(".held.se-recycle-")
                    && !rel.contains("linked")
                    && !rel.contains("secret"),
                "{path}: reported outside the rights: {event:?}"
            );
        }
        assert_eq!(summary.hits, 3, "{path}");
        assert_eq!(summary.files_matched, 2, "{path}");
        assert_eq!(summary.skipped, 0, "{path}: {events:?}");
        assert!(!summary.limited, "{path}");
        // Every regular file of the export the rights admit, and no other.
        assert_eq!(summary.files_searched, 3, "{path}");
    }
}

#[test]
fn content_search_task_host_limit_holds_over_the_whole_search() {
    let (_directory, access) = fixture();
    let mut query = ContentQuery::text("needle");
    query.max_hits = 2;
    let (events, summary) = search(&access, "/Docs", &query);
    assert_eq!(hits(&events).values().sum::<u64>(), 2, "{events:?}");
    assert_eq!(summary.hits, 2);
    assert!(summary.limited);
    // The event that reached the limit ends its file.
    assert!(events
        .iter()
        .any(|event| matches!(event, FsContentEvent::Hits { last: true, .. })));
}

#[test]
fn content_search_task_host_search_ends_with_its_stream() {
    let (_directory, access) = fixture();
    let query = ContentQuery::text("needle");
    // The client went away before the host started.
    let cancel = Arc::new(AtomicBool::new(true));
    let (tx, mut rx) = mpsc::channel(1024);
    let error = run(
        request("/Docs", &query),
        access.clone(),
        tx,
        cancel,
        Arc::new(Totals::default()),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(rx.try_recv().is_err(), "nothing is sent after the end");
    // The stream's receiver is gone: no portion and no totals can be sent.
    let (tx, rx) = mpsc::channel(1024);
    drop(rx);
    let error = run(
        request("/Docs", &query),
        access,
        tx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(Totals::default()),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn mcp_coding_task_host_search_applies_ignore_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap().join("Repo");
    let write = |rel: &str, bytes: &[u8]| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    write(".git/info/exclude", b"local.txt\n");
    write(".gitignore", b"target/\n*.log\n");
    write(".ignore", b"secret.txt\n");
    write("sub/.gitignore", b"x.txt\n");
    for rel in [
        "a.txt",
        "target/b.txt",
        "c.log",
        "secret.txt",
        "local.txt",
        "sub/x.txt",
        "sub/y.txt",
    ] {
        write(rel, NEEDLE);
    }
    let access = FsAccess::dynamic(ShareExportConfig {
        roots: vec![SharedRoot::new("Repo", text(&root)).with_access(ExportAccess::ReadOnly)],
        ..Default::default()
    });
    let mut query = ContentQuery::text("needle");
    query.respect_ignore_files = true;
    let files = |path: &str, query: &ContentQuery| {
        hits(&search(&access, path, query).0)
            .into_keys()
            .collect::<Vec<_>>()
    };
    assert_eq!(files("/Repo", &query), ["a.txt", "sub/y.txt"]);
    assert_eq!(files("/", &query), ["Repo/a.txt", "Repo/sub/y.txt"]);
    // Below the work tree root the root's files apply as well.
    assert_eq!(files("/Repo/sub", &query), ["y.txt"]);
    write("sub/target/z.txt", NEEDLE);
    assert_eq!(
        files("/Repo/sub", &query),
        ["y.txt"],
        "`target/` of the root"
    );
    query.respect_ignore_files = false;
    assert_eq!(
        files("/Repo", &query).len(),
        8,
        "without the flag every file"
    );
    // The flag travels on the wire and back.
    let mut flagged = ContentQuery::text("x");
    flagged.respect_ignore_files = true;
    let wire = FsContentQuery::from(&flagged);
    assert!(wire.to_query().unwrap().respect_ignore_files);
    assert!(serde_json::to_string(&wire)
        .unwrap()
        .contains("\"respect_ignore_files\":true"));
    assert!(
        !serde_json::to_string(&FsContentQuery::from(&ContentQuery::text("x")))
            .unwrap()
            .contains("respect_ignore_files")
    );
    let host = crate::share::wire::FsHostFeatures::host();
    assert!(host.content_search_ignore_v1);
    assert!(host.names().contains(&"content_search_ignore_v1"));
}
