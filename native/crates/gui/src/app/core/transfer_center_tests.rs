//! The transfer list model: order, ids, purposes, the finished list,
//! "Fehlende übertragen", target connections and external hand-overs.
use super::super::transfer_route::{paste_job, TransferPlace, TransferSelection};
use super::super::transfer_test_support::{done, remote, FakeLaunch};
use super::*;
use crate::transfer::ExternalSnapshot;
use crate::types::CopyMode;

fn upload_job(dir: &str) -> TransferJob {
    let selection = TransferSelection::roots(
        TransferPlace::local(),
        vec!["/data/a".to_string(), "/data/b".to_string()],
        Some("/data".to_string()),
    );
    let target = TransferPlace::remote(remote("sftp://a@one:22"), "A");
    paste_job(&selection, &target, dir, CopyMode::Copy).expect("upload job")
}

fn submit(center: &mut TransferCenter, launch: &mut FakeLaunch, purpose: TransferPurpose) -> u64 {
    center
        .submit(upload_job("/in"), purpose, &mut |request| {
            launch.launch(request)
        })
        .expect("submitted")
}

#[test]
fn transfer_engine_task_list_keeps_ids_order_and_purpose() {
    let mut center = TransferCenter::default();
    let mut launch = FakeLaunch::default();
    let first = submit(&mut center, &mut launch, TransferPurpose::Copy);
    let provide = TransferPurpose::Provide {
        temp: std::path::PathBuf::from("/tmp/e1/bereitgestellt"),
        sequence: Some(9),
    };
    let second = submit(&mut center, &mut launch, provide.clone());
    assert_ne!(first, second);
    assert_eq!(center.active_id(0), Some(first));
    assert_eq!(center.active_id(1), Some(second));
    assert_eq!(center.running_count(), 2);
    assert!(!center.is_idle());

    // The later transfer ends first; the earlier one keeps running.
    launch.senders[1]
        .send(done(2, 2, false, Vec::new(), Vec::new()))
        .expect("send");
    assert_eq!(center.poll(), vec![second]);
    assert_eq!(center.active_id(0), Some(first));
    let entry = center.finished_entry(second).expect("finished");
    assert_eq!(entry.purpose, provide);
    assert!(!entry.incomplete());
    assert!(entry.job.is_some());

    launch.senders[0]
        .send(done(1, 2, false, Vec::new(), Vec::new()))
        .expect("send");
    assert_eq!(center.poll(), vec![first]);
    assert!(center.is_idle());
    let order: Vec<u64> = center.finished.iter().map(|entry| entry.id).collect();
    assert_eq!(order, vec![first, second], "newest first");
    assert!(center
        .finished_entry(first)
        .is_some_and(FinishedEntry::incomplete));
    assert_eq!(center.poll(), Vec::<u64>::new());
}

#[test]
fn transfer_engine_task_worker_without_result_is_listed_as_failed() {
    let mut center = TransferCenter::default();
    let mut launch = FakeLaunch::default();
    let id = submit(&mut center, &mut launch, TransferPurpose::Copy);
    launch.senders.clear();
    assert_eq!(center.poll(), vec![id]);
    let entry = center.finished_entry(id).expect("listed");
    assert!(entry.failure.is_some());
    assert!(entry.can_resume(), "the job can still be completed");
}

#[test]
fn transfer_engine_task_resume_builds_the_job_with_the_resolved_roots() {
    let mut center = TransferCenter::default();
    let mut launch = FakeLaunch::default();
    let canceled = submit(&mut center, &mut launch, TransferPurpose::Copy);
    let complete = submit(&mut center, &mut launch, TransferPurpose::Copy);
    let roots = vec![ResolvedRoot {
        source: "/data/a".to_string(),
        rel: "a (2)".to_string(),
    }];
    let issue = TransferIssue {
        path: "/data/b/x".to_string(),
        message: "Zugriff verweigert".to_string(),
    };
    launch.senders[0]
        .send(done(1, 3, true, vec![issue], roots.clone()))
        .expect("send");
    launch.senders[1]
        .send(done(2, 2, false, Vec::new(), Vec::new()))
        .expect("send");
    center.poll();
    assert!(center
        .finished_entry(canceled)
        .is_some_and(FinishedEntry::can_resume));
    assert!(center
        .finished_entry(complete)
        .is_some_and(|entry| !entry.can_resume()));
    assert!(center.take_resume(complete).is_none());

    let provide = submit(
        &mut center,
        &mut launch,
        TransferPurpose::Provide {
            temp: std::path::PathBuf::from("/tmp/e2/bereitgestellt"),
            sequence: None,
        },
    );
    launch.senders[2]
        .send(done(0, 1, true, Vec::new(), Vec::new()))
        .expect("send");
    center.poll();
    assert!(
        center
            .finished_entry(provide)
            .is_some_and(|entry| !entry.can_resume()),
        "an abandoned provide download starts afresh"
    );

    let job = center.take_resume(canceled).expect("resume job");
    assert_eq!(job.resume, Some(roots));
    assert_eq!(job.target_dir, "/in");
    assert_eq!(job.conflict, crate::types::Conflict::Rename);
    assert!(
        center.finished_entry(canceled).is_none(),
        "the new run replaces the row"
    );
}

#[test]
fn transfer_engine_task_finished_list_keeps_the_newest_thirty() {
    let mut center = TransferCenter::default();
    let ids: Vec<u64> = (0..FINISHED_KEPT + 5)
        .map(|index| {
            center.record_failure(
                TransferKind::Upload,
                "/data".to_string(),
                format!("Ziel {index}"),
                "Ziel nicht erreichbar".to_string(),
            )
        })
        .collect();
    center.trim_finished();
    assert_eq!(center.finished.len(), FINISHED_KEPT);
    assert_eq!(
        center.finished.front().map(|entry| entry.id),
        ids.last().copied()
    );
    assert!(center.finished_entry(ids[0]).is_none(), "oldest dropped");
    let newest = ids[ids.len() - 1];
    center.remove_finished(newest);
    assert!(center.finished_entry(newest).is_none());
    center.clear_finished();
    assert_eq!(center.total_count(), 0);
}

#[test]
fn transfer_engine_task_connecting_target_opens_or_fails_clearly() {
    let mut center = TransferCenter::default();
    let selection = TransferSelection::roots(
        TransferPlace::remote(remote("sftp://a@one:22"), "A"),
        vec!["/docs/x".to_string()],
        None,
    );
    let (ready_tx, ready_rx) = crossbeam_channel::unbounded();
    let (failed_tx, failed_rx) = crossbeam_channel::unbounded();
    let (_, lost_rx) = crossbeam_channel::unbounded();
    let target = remote("webdav://b@two:443");
    center.connect_target(selection.clone(), "B".into(), "B: /docs".into(), ready_rx);
    center.connect_target(selection.clone(), "B".into(), "B: /weg".into(), failed_rx);
    center.connect_target(selection, "C".into(), "C: /".into(), lost_rx);
    assert_eq!(center.running_count(), 3);
    assert_eq!(
        center.poll_connecting().len(),
        1,
        "the dropped sender ends at once"
    );
    ready_tx
        .send(Ok((target.clone(), "/docs".to_string())))
        .expect("send");
    failed_tx
        .send(Err("Zeitüberschreitung".to_string()))
        .expect("send");
    let outcomes = center.poll_connecting();
    assert_eq!(outcomes.len(), 2);
    assert!(center.connecting.is_empty());
    for outcome in outcomes {
        match outcome {
            ConnectOutcome::Ready {
                target: place,
                target_dir,
                ..
            } => {
                assert_eq!(target_dir, "/docs");
                assert_eq!(place.label, "B");
                assert!(place
                    .backend()
                    .is_some_and(|backend| Arc::ptr_eq(backend, &target)));
            }
            ConnectOutcome::Failed {
                target_label,
                message,
                ..
            } => {
                assert_eq!(target_label, "B: /weg");
                assert_eq!(message, "Zeitüberschreitung");
            }
        }
    }
}

#[test]
fn transfer_engine_task_external_handover_stays_listed_after_it_ends() {
    let snapshot = |id: u64, files_done: u64, finished: bool| ExternalSnapshot {
        id,
        label: "Explorer: 3 Einträge".to_string(),
        files_total: 3,
        files_done,
        bytes_done: files_done * 10,
        errors: 0,
        elapsed_ms: 5,
        finished,
        note: Some("Auswahl zu groß für den Explorer".to_string()),
        issues: Vec::new(),
    };
    let mut center = TransferCenter::default();
    assert!(center
        .sync_externals(vec![snapshot(5, 1, false)])
        .is_empty());
    assert_eq!(center.externals.len(), 1);
    assert!(!center.externals[0].ended());
    assert_eq!(center.running_count(), 1);
    // The provider dropped its handle: counts and note stay visible, and the
    // end is reported once.
    let ended = center.sync_externals(Vec::new());
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id, 5);
    assert_eq!(center.externals.len(), 1);
    assert!(center.externals[0].ended());
    assert_eq!(center.externals[0].snapshot.files_done, 1);
    assert!(center.externals[0].snapshot.note.is_some());
    assert!(center.sync_externals(Vec::new()).is_empty());
    // A hand-over first seen already finished ends at once.
    let ended = center.sync_externals(vec![snapshot(6, 3, true)]);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id, 6);
    assert_eq!(center.externals[0].snapshot.id, 6, "newest first");
    center.trim_finished();
    assert_eq!(center.externals.len(), 2);
    center.remove_external(5);
    assert_eq!(center.externals.len(), 1);
    center.clear_finished();
    assert!(center.externals.is_empty());
}
