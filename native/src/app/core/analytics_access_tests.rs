use super::*;
#[test]
fn analytics_access_task_remote_access_never_requests_local_privileges() {
    let source = StorageScanSource::remote(
        std::sync::Arc::new(crate::vfs::LocalBackend::new("/")),
        "/remote",
        "fixture",
    );
    assert!(!access_offer(5, Some(&source), |_| panic!(
        "remote scan must not query local authority"
    )));
    let local = StorageScanSource::local("C:/");
    assert!(access_offer(1, Some(&local), |root| root == "C:/"));
    assert!(!access_offer(0, Some(&local), |_| panic!("no denial")));
    assert!(!should_log_failure(StorageRunState::Partial, false));
    assert!(!should_log_failure(StorageRunState::Failed, true));
    assert!(should_log_failure(StorageRunState::Failed, false));
}
#[test]
fn analytics_access_task_report_lists_each_retained_path_and_omissions() {
    let issues: Vec<_> = (0..24)
        .map(|index| ScanIssue {
            path: format!("C:/blocked-{index}/"),
            detail: "Zugriff verweigert".into(),
        })
        .collect();
    let report = issue_report(&issues, 3, 27);
    for issue in &issues {
        assert!(report.contains(&issue.path));
    }
    assert!(report.contains("3 weitere Probleme"));
    assert!(report.contains("27 Zugriff verweigert"));
}
#[test]
fn analytics_access_task_new_scan_resets_prompt_without_canceling_consented_launch() {
    let (_tx, rx) = crossbeam_channel::bounded(1);
    let mut state = AnalyticsAccess {
        permission_denied: 3,
        offer: true,
        pending: Some(rx),
        message: Some("previous".into()),
        ..Default::default()
    };
    state.reset_scan();
    assert_eq!(state.permission_denied, 0);
    assert!(!state.offer);
    assert!(state.message.is_none());
    assert!(state.pending.is_some());
}
