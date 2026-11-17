//! Commit waits without a time limit (user rule 2026-11-17): the answer
//! ends the wait whenever it comes, the transfer's cancellation still ends
//! it, and an unconfirmed batch is asked about as long as the host reports
//! it pending.
use std::io;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::{run_until_answer, waits_for_host_commit, OutcomeWait};
use crate::share::wire::{FsRequest, FsStageFinish, FsSyncFilesystem};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime")
}

#[test]
fn zeitlimit_task_a_commit_wait_ends_with_the_answer_or_the_cancellation_only() {
    let runtime = runtime();
    // An answer later than several cancellation looks still arrives.
    let answer = runtime.block_on(run_until_answer("test commit", async {
        tokio::time::sleep(Duration::from_millis(700)).await;
        Ok::<_, io::Error>(7)
    }));
    assert_eq!(answer.expect("the late answer"), 7);
    // The host's error is the error of the wait.
    let refused = runtime.block_on(run_until_answer("test commit", async {
        Err::<(), _>(io::Error::new(io::ErrorKind::PermissionDenied, "refused"))
    }));
    assert_eq!(
        refused.expect_err("the refusal").kind(),
        io::ErrorKind::PermissionDenied
    );
    // A cancelled transfer ends a wait that would never get an answer.
    let cancel = AtomicBool::new(true);
    let canceled = crate::source_media::with_cancellation(&cancel, || {
        runtime.block_on(run_until_answer(
            "test commit",
            std::future::pending::<io::Result<()>>(),
        ))
    });
    let error = canceled.expect_err("a cancelled wait");
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(error.to_string().contains("test commit"), "{error}");
    // Without a cancelled transfer the same wait does not end on its own.
    let running = AtomicBool::new(false);
    let waited = crate::source_media::with_cancellation(&running, || {
        runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_millis(600),
                run_until_answer("test commit", std::future::pending::<io::Result<()>>()),
            )
            .await
        })
    });
    assert!(waited.is_err(), "the wait ended by itself: {waited:?}");
}

#[test]
fn zeitlimit_task_commit_requests_and_pending_batches_have_no_time_limit() {
    assert!(waits_for_host_commit(&FsRequest::WriteDone));
    assert!(waits_for_host_commit(&FsRequest::FinishStage(
        FsStageFinish::default()
    )));
    assert!(waits_for_host_commit(&FsRequest::SyncFilesystem(
        FsSyncFilesystem::default()
    )));
    for other in [
        FsRequest::Stat { path: "a".into() },
        FsRequest::ListDir { path: "a".into() },
        FsRequest::PutBatchStatus {
            nonce: "0".repeat(32),
        },
    ] {
        assert!(!waits_for_host_commit(&other), "{other:?}");
    }

    let start = Instant::now();
    let at = |seconds: u64| start + Duration::from_secs(seconds);
    let mut wait = OutcomeWait::new(start);
    assert!(wait.ask_again(at(59)));
    // Every "pending" over an hour keeps the asking going.
    for minute in 1..=60u64 {
        wait.pending(at(minute * 55));
        assert!(wait.ask_again(at(minute * 55 + 59)), "minute {minute}");
    }
    // One stall window without a sign of life ends it.
    assert!(!wait.ask_again(at(60 * 55 + OutcomeWait::STALL.as_secs())));
}
