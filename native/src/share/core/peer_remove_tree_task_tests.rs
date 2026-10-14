use super::*;

fn progress(applying: bool, planned: u64, removed: u64) -> FsRemoveTreeMessage {
    FsRemoveTreeMessage::Progress {
        applying,
        planned,
        removed,
    }
}

#[test]
fn delete_speed_task_peer_progress_maps_phase_counters_and_done() {
    let mut reported = Reported::default();
    let mut seen = Vec::new();
    let mut record = |update: RecursiveDeleteProgress| seen.push(update);
    for message in [progress(false, 3, 0), progress(true, 5, 2)] {
        assert!(reported
            .apply("/Docs/old", message, &mut record)
            .unwrap()
            .is_none());
    }
    let report = reported
        .apply(
            "/Docs/old",
            FsRemoveTreeMessage::Done {
                planned: 5,
                removed: 5,
            },
            &mut record,
        )
        .unwrap()
        .expect("done closes the stream");
    assert_eq!(
        report,
        RecursiveDeleteReport {
            status: RecursiveDeleteStatus::Complete,
            planned: 5,
            removed: 5,
        }
    );
    assert_eq!(
        seen,
        [
            RecursiveDeleteProgress {
                phase: RecursiveDeletePhase::Planning,
                planned: 3,
                removed: 0,
                current: "/Docs/old".into(),
            },
            RecursiveDeleteProgress {
                phase: RecursiveDeletePhase::Applying,
                planned: 5,
                removed: 2,
                current: "/Docs/old".into(),
            },
        ]
    );
}

#[test]
fn delete_speed_task_peer_rejects_counters_going_backwards() {
    for later in [progress(true, 4, 2), progress(true, 5, 1)] {
        let mut reported = Reported::default();
        let mut calls = 0;
        let mut record = |_: RecursiveDeleteProgress| calls += 1;
        reported
            .apply("/Docs/old", progress(true, 5, 2), &mut record)
            .unwrap();
        let error = reported.apply("/Docs/old", later, &mut record).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(calls, 1, "a rejected message reports no progress");
    }
}

#[test]
fn delete_speed_task_peer_rejects_done_below_progress() {
    for done in [
        FsRemoveTreeMessage::Done {
            planned: 4,
            removed: 4,
        },
        FsRemoveTreeMessage::Done {
            planned: 6,
            removed: 2,
        },
    ] {
        let mut reported = Reported::default();
        let mut record = |_: RecursiveDeleteProgress| {};
        reported
            .apply("/Docs/old", progress(true, 5, 3), &mut record)
            .unwrap();
        let error = reported.apply("/Docs/old", done, &mut record).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn delete_speed_task_peer_cancel_becomes_canceled_report_with_last_counters() {
    let mut reported = Reported::default();
    reported
        .apply("/Docs/old", progress(true, 9, 4), &mut |_| {})
        .unwrap();
    let canceled = AtomicBool::new(true);
    let report = reported
        .finish(Err(io::ErrorKind::Interrupted.into()), &canceled)
        .unwrap();
    assert_eq!(
        report,
        RecursiveDeleteReport {
            status: RecursiveDeleteStatus::Canceled,
            planned: 9,
            removed: 4,
        }
    );
}

#[test]
fn delete_speed_task_peer_error_keeps_last_counters() {
    let mut reported = Reported::default();
    reported
        .apply("/Docs/old", progress(true, 9, 4), &mut |_| {})
        .unwrap();
    // An interruption the caller did not ask for is a failure, like any
    // host error (revoked right, protected entry, lost connection).
    let running = AtomicBool::new(false);
    for error in [
        io::Error::from(io::ErrorKind::Interrupted),
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Pfad ist nicht freigegeben",
        ),
        io::Error::new(io::ErrorKind::NotConnected, "connection lost"),
    ] {
        let kind = error.kind();
        let failure = reported.finish(Err(error), &running).unwrap_err();
        assert_eq!(failure.error.kind(), kind);
        assert_eq!((failure.planned, failure.removed), (9, 4));
    }
    let canceled = AtomicBool::new(true);
    let failure = reported
        .finish(
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "revoked")),
            &canceled,
        )
        .unwrap_err();
    assert_eq!(failure.error.kind(), io::ErrorKind::PermissionDenied);
    let complete = RecursiveDeleteReport {
        status: RecursiveDeleteStatus::Complete,
        planned: 9,
        removed: 9,
    };
    assert_eq!(reported.finish(Ok(complete), &running).unwrap(), complete);
}
