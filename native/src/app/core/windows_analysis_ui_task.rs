use super::{AnalyticsScan, App, StorageRunState, StorageScanSource};
use crate::analytics::{Progress, ScanOutcome};
use std::{sync::atomic::Ordering, time::Instant};

#[test]
fn windows_remote_task_analysis_ui_keeps_evidence_until_worker_completion() {
    let mut app = App::new_for_copy_task();
    let install = |app: &mut App| {
        let (send, rx) = crossbeam_channel::bounded(1);
        let progress = Progress::default();
        progress.files.store(128, Ordering::Relaxed);
        progress.dirs.store(3, Ordering::Relaxed);
        progress.bytes.store(4096, Ordering::Relaxed);
        app.analytics_source = Some(StorageScanSource::local("C:/task"));
        app.analytics_scan = Some(AnalyticsScan {
            rx, progress, root: "C:/task".into(), started: Instant::now(),
        });
        app.analytics_state = StorageRunState::Running;
        app.analytics_totals = None;
        send
    };

    let worker = install(&mut app);
    app.cancel_analytics_scan();
    app.poll_analytics_scan();
    assert_eq!(app.analytics_state, StorageRunState::Running);
    let pending = app.analytics_scan.as_ref().expect("pending cancellation retains the worker");
    assert!(pending.progress.cancel.load(Ordering::Relaxed));
    assert_eq!(pending.progress.snapshot().files, 128);
    worker.send(ScanOutcome::canceled()).unwrap();
    app.poll_analytics_scan();
    assert_eq!(app.analytics_state, StorageRunState::Canceled);
    assert!(app.analytics_scan.is_none());
    let totals = &app.analytics_totals.as_ref().unwrap().0;
    assert_eq!((totals.files, totals.dirs, totals.bytes), (128, 3, 4096));

    let worker = install(&mut app);
    drop(worker);
    app.poll_analytics_scan();
    assert_eq!(app.analytics_state, StorageRunState::Failed);
    let totals = &app.analytics_totals.as_ref().unwrap().0;
    assert_eq!((totals.files, totals.dirs, totals.bytes), (128, 3, 4096));
    assert!(app.analytics_issues.iter().any(|issue| issue.detail.contains("ohne Ergebnis")));
}
