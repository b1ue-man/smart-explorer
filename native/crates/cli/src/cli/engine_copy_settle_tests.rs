//! Acceptance signals of the result rules and the `--stats` line of the
//! engine path (U49), against constructed engine results.
use super::{finished, issue, Scratch, LINK_TEXT, SPECIAL_TEXT};
use crate::cli::engine_copy::{parse_log, settle, stats_line};
use crate::cli::tree_observer::{Counter, TreeObserver};
use crate::transfer::{ResolvedRoot, TransferIssue};

#[test]
fn speed_task_cli_engine_failure_lists_issues_and_fails() {
    let issues: Vec<TransferIssue> = (1..=7)
        .map(|n| issue(&format!("/src/file{n}"), "Zugriff verweigert"))
        .collect();
    let mut run = finished(issues);
    run.progress.log_path = Some("/logs/transfer.jsonl".to_string());
    let error = settle(&run, "/dst").unwrap_err();
    let lines: Vec<&str> = error.lines().collect();
    assert_eq!(lines[0], "7 files failed");
    assert_eq!(lines[1], "/src/file1: Zugriff verweigert");
    assert_eq!(lines[5], "/src/file5: Zugriff verweigert");
    assert!(
        !error.contains("/src/file6: "),
        "only the first five are listed"
    );
    assert_eq!(lines[6], "all issues: /logs/transfer.jsonl");

    // Omissions among failures stay omissions and are not counted as failed.
    let mixed = finished(vec![
        issue("/src/link", LINK_TEXT),
        issue("/src/broken", "Zugriff verweigert"),
        issue("", "Zielordner nicht beschreibbar"),
    ]);
    let error = settle(&mixed, "/dst").unwrap_err();
    assert!(error.starts_with("2 files failed"));
    assert!(error.contains("/src/broken: Zugriff verweigert"));
    assert!(error.contains("\nZielordner nicht beschreibbar"));
    assert!(!error.contains("/src/link"));
}

#[test]
fn speed_task_cli_engine_issue_log_completes_the_shown_issues() {
    let text = "{\"path\":\"/a\",\"message\":\"Spezialdatei wird nicht übertragen\"}\n\
                not json\n\
                {\"vault\":\"v\",\"path_sealed\":\"x\",\"message_sealed\":\"y\"}\n\
                {\"path\":\"/b\",\"message\":\"Zugriff verweigert\"}\n";
    let parsed = parse_log(text);
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0], issue("/a", SPECIAL_TEXT));
    assert!(parsed[1].path.is_empty() && parsed[1].message.contains("sealed"));

    // 120 issues, 100 shown: the rest comes from the log file.
    let scratch = Scratch::new("log");
    let mut lines = String::new();
    for n in 0..120 {
        let message = if n % 2 == 0 {
            LINK_TEXT
        } else {
            "Zugriff verweigert"
        };
        lines.push_str(&format!(
            "{{\"path\":\"/src/{n}\",\"message\":\"{message}\"}}\n"
        ));
    }
    let log = scratch.file("transfer.jsonl", lines.as_bytes());
    let shown: Vec<TransferIssue> = (0..100)
        .map(|n| {
            let message = if n % 2 == 0 {
                LINK_TEXT
            } else {
                "Zugriff verweigert"
            };
            issue(&format!("/src/{n}"), message)
        })
        .collect();
    let mut run = finished(shown.clone());
    run.progress.errors = 120;
    run.progress.log_path = Some(log);
    let error = settle(&run, "/dst").unwrap_err();
    assert!(error.starts_with("60 files failed"), "{error}");

    // Without a readable log the unseen issues count as failures.
    let mut unreadable = finished(shown);
    unreadable.progress.errors = 120;
    unreadable.progress.log_path = Some(scratch.path("missing.jsonl"));
    let error = settle(&unreadable, "/dst").unwrap_err();
    assert!(error.starts_with("70 files failed"), "{error}");
}

#[test]
fn speed_task_cli_engine_skipped_existing_file_is_a_failure() {
    let mut run = finished(Vec::new());
    run.progress.skipped = 2;
    assert_eq!(
        settle(&run, "/dst").unwrap_err(),
        "destination changed during copy: 2 existing files were left alone"
    );
}

#[test]
fn speed_task_cli_engine_numbered_destination_is_a_failure() {
    let mut run = finished(Vec::new());
    run.roots = vec![ResolvedRoot {
        source: "/src/report.txt".to_string(),
        rel: "report (2).txt".to_string(),
    }];
    let error = settle(&run, "/dst/report.txt").unwrap_err();
    assert!(
        error.contains("report.txt appeared at the destination"),
        "{error}"
    );
    assert!(error.contains("report (2).txt"));
    run.roots[0].rel = "report.txt".to_string();
    assert!(settle(&run, "/dst/report.txt").is_ok());
}

#[test]
fn speed_task_cli_stats_line_texts() {
    let observer = TreeObserver::default();
    observer.begin_engine();
    observer.set_engine_done(3, 3 * 1024 * 1024);
    observer.set_engine_elapsed(1_500);
    let cases = [
        ("link", "link"),
        ("remote", "server or link"),
        ("source", "source disk"),
        ("target", "target disk"),
        ("se_cpu", "se cpu"),
        ("se_thread", "se threads"),
        ("memory", "memory"),
        ("none", "none"),
        ("ramp", "none"),
        ("idle", "none"),
    ];
    for (code, text) in cases {
        observer.set_engine_stats(0, code, None, 0);
        assert_eq!(
            stats_line(&observer.progress(), 9_999),
            format!("transferred 3 files, 3.00 MB in 1.5 s (2.00 MB/s); limited by: {text}"),
            "limiter code {code}"
        );
    }
}

#[test]
fn speed_task_cli_stats_line_for_the_legacy_path() {
    let observer = TreeObserver::default();
    let mut reader = observer.reader(&[0u8; 2048][..], Counter::Written);
    std::io::copy(&mut reader, &mut std::io::sink()).expect("copy");
    assert_eq!(
        stats_line(&observer.progress(), 2_000),
        "transferred 2.00 KB in 2.0 s (1.00 KB/s); limited by: unknown (legacy path)"
    );
}
