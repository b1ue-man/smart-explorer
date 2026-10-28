//! Group commits (G4 MS5): reporting workers never wait for a flush, the
//! committer records finished work while the run goes on, and a stop is
//! visible to admission at once.
use super::super::checkpoint_review_tests::Fixture;
use super::super::pair_lock::PairLock;
use super::super::types::Sig;
use super::*;
use std::sync::mpsc;
use std::time::Duration;

fn copied(rel: &str) -> CompletedAction {
    let sig = Sig {
        size: 4,
        mtime_ms: 1_000,
        hash: 0,
    };
    CompletedAction {
        rel: rel.into(),
        kind: CompletedKind::Copied { from: PairSide::A },
        src_sig: Some(sig),
        dst_sig: Some(sig),
        durable: true,
    }
}

#[test]
fn speed_task_sync_workers_never_wait_for_a_group_commit() {
    let fixture = Fixture::new();
    let lock = PairLock::acquire(&fixture.key.lock_id).unwrap();
    let sink = CheckpointSink::new(
        fixture.endpoints(),
        &lock,
        &fixture.key,
        KeyPolicy::default(),
        None,
    )
    .unwrap();
    sink.during(|| {
        // A flush in progress holds the commit state.
        let held = sink.lock();
        let (done, finished) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                assert!(!sink.should_stop());
                sink.completed(copied("one"));
                sink.stopped(RunStop::TargetFull { side: PairSide::B });
                assert!(sink.should_stop());
                let _ = done.send(());
            });
            let answered = finished.recv_timeout(Duration::from_secs(30));
            drop(held);
            assert!(answered.is_ok(), "a worker waited for the flush");
        });
    });
    let result = sink.finish();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(result.baseline.contains_key("one"));
    assert!(result.stopped.is_some());
}

#[test]
fn speed_task_sync_group_commit_records_finished_work_while_the_run_continues() {
    let fixture = Fixture::new();
    let lock = PairLock::acquire(&fixture.key.lock_id).unwrap();
    let sink = CheckpointSink::new(
        fixture.endpoints(),
        &lock,
        &fixture.key,
        KeyPolicy::default(),
        None,
    )
    .unwrap();
    sink.during(|| {
        sink.completed(copied("early"));
        let mut recorded = false;
        for _ in 0..200 {
            let (_, stored, _) = Journal::load(&fixture.key, KeyPolicy::default()).unwrap();
            if stored.baseline.contains_key("early") {
                recorded = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(recorded, "the committer journals without a 64-action batch");
    });
    assert!(sink.finish().baseline.contains_key("early"));
}

#[test]
fn speed_task_sync_queue_beyond_its_bound_is_committed_by_the_worker() {
    let fixture = Fixture::new();
    let lock = PairLock::acquire(&fixture.key.lock_id).unwrap();
    let sink = CheckpointSink::new(
        fixture.endpoints(),
        &lock,
        &fixture.key,
        KeyPolicy::default(),
        None,
    )
    .unwrap();
    sink.during(|| {
        // However slow the committer is, the queue never outgrows its bound.
        for index in 0..INLINE_ACTIONS {
            sink.completed(copied(&format!("f{index}")));
        }
        assert!(sink.inbox().queued.len() < INLINE_ACTIONS);
    });
    assert_eq!(sink.finish().baseline.len(), INLINE_ACTIONS);
}
