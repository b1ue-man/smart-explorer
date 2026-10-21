//! R15c: the scanner keeps every failed path (formerly the first 500) and
//! sends each path once.
use super::*;

#[test]
fn drive_task_scanner_records_more_than_500_failed_paths() {
    let failed = Mutex::new(Vec::new());
    for index in 0..1200 {
        record_failure(
            &failed,
            &format!("/dir{index}"),
            "Zugriff verweigert".into(),
        );
    }
    let recorded = failed.lock().unwrap();
    assert_eq!(recorded.len(), 1200);
    assert_eq!(recorded[1199].0, "/dir1199");
    let sent = distinct_failures(&recorded);
    assert_eq!(sent.len(), 1200);
}

#[test]
fn drive_task_scanner_sends_each_failed_path_once() {
    let failed = Mutex::new(Vec::new());
    record_failure(&failed, "/a", "erste Meldung".into());
    record_failure(&failed, "/b", "Zugriff verweigert".into());
    record_failure(&failed, "/a", "zweite Meldung".into());
    let sent = distinct_failures(&failed.lock().unwrap());
    assert_eq!(
        sent,
        vec![
            ("/a".to_string(), "erste Meldung".to_string()),
            ("/b".to_string(), "Zugriff verweigert".to_string()),
        ]
    );
}
