use super::*;
use crossbeam_channel::unbounded;

#[test]
fn search_recursive_access_task_remote_over_budget_directory_retains_prefix() {
    let (tx, rx) = unbounded();
    let truncated = Arc::new(AtomicBool::new(false));
    let mut state = WalkState::new(
        tx,
        Arc::new(AtomicBool::new(false)),
        truncated.clone(),
        None,
        Instant::now(),
    );
    state.budget = ScanBudget::with_limits(3, 8192, 512);
    let files = (0..8)
        .map(|index| VfsMeta {
            name: format!("file-{index}.blend"),
            size: 7,
            ..Default::default()
        })
        .collect();
    assert!(!state.process_listing(
        &PendingRemoteDir::root("/root".into()),
        files,
        true,
        &mut Vec::new()
    ));
    state.finish();
    let mut retained = Vec::new();
    let mut done = false;
    for message in rx.try_iter() {
        match message {
            ScanMessage::Entries(entries) => retained.extend(entries),
            ScanMessage::Done(progress) => {
                done = true;
                assert!(progress.errors > 0);
            }
            _ => {}
        }
    }
    assert!(done && truncated.load(Ordering::Relaxed));
    assert_eq!(
        retained.len(),
        3,
        "a large directory must not discard its whole prefix"
    );
}

#[test]
fn search_recursive_access_task_remote_failed_branch_keeps_readable_sibling() {
    let (tx, rx) = unbounded();
    let mut state = WalkState::new(
        tx,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        None,
        Instant::now(),
    );
    state.listing_failed(
        "/root/denied",
        io::Error::from(io::ErrorKind::PermissionDenied),
    );
    assert!(state.process_listing(
        &PendingRemoteDir::root("/root/readable".into()),
        vec![VfsMeta {
            name: "match.blend".into(),
            ..Default::default()
        }],
        true,
        &mut Vec::new()
    ));
    state.finish();
    let mut names = Vec::new();
    for message in rx.try_iter() {
        if let ScanMessage::Entries(entries) = message {
            names.extend(entries.into_iter().map(|entry| entry.name.to_string()));
        }
    }
    assert_eq!(names, ["match.blend"]);
}
