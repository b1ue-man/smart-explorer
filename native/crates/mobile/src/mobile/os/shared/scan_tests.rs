//! R15c in `scan.*`: no count of issues, no cap of a view window, and the
//! session retention drops only finished scans, oldest first.
use super::*;

fn session(finished: bool) -> Arc<ScanSession> {
    Arc::new(ScanSession {
        base: Loc::parse("/storage/emulated/0").expect("local location"),
        filter: pass_all(false),
        data: Mutex::new(ScanData::default()),
        finished: AtomicBool::new(finished),
    })
}

fn ids(registry: &ScanRegistry) -> Vec<String> {
    lock(&registry.sessions)
        .iter()
        .map(|(id, _)| id.clone())
        .collect()
}

#[test]
fn drive_task_scan_keeps_more_than_500_issues() {
    let mut data = ScanData::default();
    for index in 0..1200 {
        push_issue(
            &mut data,
            format!("/dir{index}"),
            "Zugriff verweigert".into(),
        );
    }
    push_issue(&mut data, "/dir7".into(), "Zugriff verweigert".into());
    push_issue(&mut data, String::new(), "Wurzel nicht lesbar".into());
    assert_eq!(data.issues.len(), 1201);
    assert_eq!(data.issues[1199].0, "/dir1199");
    assert_eq!(data.issues[1200].1, "Wurzel nicht lesbar");
}

#[test]
fn drive_task_scan_view_returns_the_requested_amount() {
    assert_eq!(view_limit(&json!({ "limit": 2000 })), 2000);
    assert_eq!(view_limit(&json!({ "limit": 1 })), 1);
    assert_eq!(view_limit(&json!({})), usize::MAX);
    let rows: Vec<u32> = (0..2500).collect();
    let page = window(&rows, 100, view_limit(&json!({ "limit": 2000 })));
    assert_eq!(page.len(), 2000);
    assert_eq!(page[0], 100);
    assert_eq!(window(&rows, 0, view_limit(&json!({}))).len(), 2500);
}

#[test]
fn drive_task_scan_new_session_drops_oldest_finished_not_running() {
    let registry = ScanRegistry::default();
    // Oldest two run, the next four are finished.
    registry.insert("run1".into(), session(false));
    registry.insert("run2".into(), session(false));
    for index in 1..=4 {
        registry.insert(format!("done{index}"), session(true));
    }
    registry.insert("new".into(), session(false));
    assert_eq!(
        ids(&registry),
        vec!["run1", "run2", "done2", "done3", "done4", "new"]
    );
    assert!(registry.get("run1").is_ok());
    assert!(registry.get("done1").is_err());
}

#[test]
fn drive_task_scan_running_sessions_all_stay() {
    let registry = ScanRegistry::default();
    for index in 0..9 {
        registry.insert(format!("run{index}"), session(false));
    }
    assert_eq!(ids(&registry).len(), 9);
    // A finished one joins: it is the only one that may go, and it goes
    // because nine running sessions already exceed the retention.
    registry.insert("done".into(), session(true));
    assert_eq!(ids(&registry).len(), 9);
    assert!(registry.get("done").is_err());
    // Once one of the running ones has finished, the next scan drops it.
    lock(&registry.sessions)[3]
        .1
        .finished
        .store(true, Ordering::Release);
    registry.insert("next".into(), session(false));
    assert_eq!(ids(&registry).len(), 9);
    assert!(registry.get("run3").is_err());
    assert!(registry.get("next").is_ok());
}

#[test]
fn drive_task_scan_task_end_marks_session_finished() {
    let scan = session(false);
    {
        let _finish = FinishOnDrop(&scan);
        assert!(!scan.finished());
    }
    assert!(scan.finished());
}
