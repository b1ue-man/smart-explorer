//! What the transfer list, the chips and the notices say.
use super::super::transfer_center::{FinishedEntry, TransferPurpose};
use super::*;
use crate::transfer::TransferIssue;

fn progress(files: (u64, u64), bytes: (u64, u64)) -> TransferProgress {
    let mut progress = TransferProgress::new(TransferKind::Download, "Download", files.1, bytes.1);
    progress.files_done = files.0;
    progress.bytes_done = bytes.0;
    progress
}

fn finished(progress: TransferProgress) -> FinishedEntry {
    FinishedEntry {
        id: 1,
        progress,
        canceled: false,
        failure: None,
        errors: Vec::new(),
        issues: Vec::new(),
        roots: Vec::new(),
        job: None,
        purpose: TransferPurpose::Copy,
        show_issues: false,
    }
}

#[test]
fn transfer_engine_task_rows_show_remaining_time_only_once_everything_was_found() {
    let mut running = progress((10, 1234), (1000, 5000));
    running.rate_bps = 1000;
    running.discovering = true;
    running.parallel = 4;
    assert_eq!(eta_secs(&running), None, "totals still grow");
    assert_eq!(running_state(&running, false), "sucht… 1.234 gefunden");
    assert!(!rate_line(&running).unwrap_or_default().contains("noch"));
    running.discovering = false;
    assert_eq!(eta_secs(&running), Some(4));
    assert!(rate_line(&running)
        .unwrap_or_default()
        .ends_with("noch 0:04"));
    assert_eq!(running_state(&running, false), "läuft");
    assert_eq!(running_state(&running, true), "wird abgebrochen…");
    running.rate_bps = 0;
    assert_eq!(eta_secs(&running), None);
    assert_eq!(rate_line(&running), None);
    let waiting = progress((0, 3), (0, 0));
    assert_eq!(running_state(&waiting, false), "wartet");
}

#[test]
fn transfer_engine_task_rows_list_counts_activity_and_tallies() {
    let mut running = progress((1234, 5678), (0, 0));
    assert_eq!(counts_line(&running), "1.234 / 5.678 Dateien · 0 B");
    running.active = vec![
        "/a/eins.txt".to_string(),
        "C:\\b\\zwei.txt".to_string(),
        "/c/drei/".to_string(),
        "/d/vier.txt".to_string(),
    ];
    running.parallel = 8;
    assert_eq!(
        activity_line(&running).as_deref(),
        Some("läuft: eins.txt, zwei.txt, drei · 8 parallel")
    );
    assert_eq!(tally_line(&running), None);
    running.skipped = 3;
    running.errors = 1234;
    assert_eq!(
        tally_line(&running).as_deref(),
        Some("übersprungen: 3 · Fehler: 1.234")
    );
    assert_eq!(kind_icon(TransferKind::Upload), "⬆");
    assert_eq!(kind_icon(TransferKind::RemoteCopy), "⇄");
}

#[test]
fn transfer_engine_task_rows_notices_follow_the_outcome() {
    let mut done = progress((1234, 1234), (2048, 2048));
    done.elapsed_ms = 83_000;
    let entry = finished(done.clone());
    assert_eq!(finished_state(&entry), "fertig");
    assert_eq!(
        completion_notice(&entry),
        "✓ 1.234 Datei(en) (2.00 KB) übertragen in 1:23"
    );

    let mut with_errors = finished(done.clone());
    with_errors.progress.errors = 3;
    assert_eq!(finished_state(&with_errors), "mit Fehlern");
    assert!(completion_notice(&with_errors).starts_with('⚠'));
    assert!(completion_notice(&with_errors).contains("mit 3 Fehler(n) – Details in Übertragungen"));

    let mut canceled = finished(done.clone());
    canceled.canceled = true;
    assert_eq!(finished_state(&canceled), "abgebrochen");
    assert!(completion_notice(&canceled).contains("Fehlende übertragen"));

    let mut failed = finished(done);
    failed.failure = Some("Ziel nicht erreichbar".to_string());
    assert_eq!(finished_state(&failed), "fehlgeschlagen");
    assert_eq!(
        completion_notice(&failed),
        "⚠ Übertragung fehlgeschlagen: Ziel nicht erreichbar"
    );
    assert_eq!(format_duration(3723), "1:02:03");
    assert_eq!(group_digits(1_234_567), "1.234.567");
    assert_eq!(group_digits(999), "999");
}

#[test]
fn transfer_engine_task_rows_issue_text_is_complete_for_copying() {
    let mut entry = finished(progress((1, 5), (0, 0)));
    entry.issues = vec![
        TransferIssue {
            path: "/a/x".to_string(),
            message: "Zugriff verweigert".to_string(),
        },
        TransferIssue {
            path: String::new(),
            message: "Verbindung verloren".to_string(),
        },
    ];
    entry.progress.errors = 150;
    entry.progress.log_path = Some("/log/transfer-1.jsonl".to_string());
    let text = issues_text(&entry);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "/a/x: Zugriff verweigert");
    assert_eq!(lines[1], "Verbindung verloren");
    assert_eq!(lines[2], "… 148 weitere Fehler (vollständig im Protokoll)");
    assert_eq!(lines[3], "Protokoll: /log/transfer-1.jsonl");
    assert_eq!(issue_count(&entry), 150);
    // A worker without structured issues still shows its lines.
    let mut legacy = finished(progress((0, 1), (0, 0)));
    legacy.errors = vec!["Datei a: weg".to_string()];
    assert_eq!(issues_text(&legacy), "Datei a: weg");
    assert_eq!(issue_count(&legacy), 1);
}

#[test]
fn transfer_engine_task_rows_title_prefers_the_engine_labels() {
    let mut reported = progress((0, 0), (0, 0));
    assert_eq!(transfer_title(&reported, None), "Download");
    reported.source = "Server: /docs".to_string();
    reported.target = "C:/Ziel".to_string();
    assert_eq!(transfer_title(&reported, None), "Server: /docs → C:/Ziel");
}
