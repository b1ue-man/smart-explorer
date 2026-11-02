//! The overload policy of sync runs: only overload is waited out, the peer's
//! delay is honored, patience ends the waiting, a cancel ends it at once.
use super::*;
use crate::transfer::FlowPermit;
use std::cell::Cell;

fn overload(retry_after: Option<Duration>) -> io::Error {
    crate::vfs::congestion_error("busy", retry_after)
}

#[test]
fn transfer_engine_task_sync_overload_backoff_waits_only_for_overload() {
    let progress = Progress::default();
    let mut backoff = Backoff::new(&progress);
    assert_eq!(
        backoff.pause(&overload(Some(Duration::from_millis(7)))),
        Some(Duration::from_millis(7))
    );
    let timed_out = io::Error::new(io::ErrorKind::TimedOut, "stalled");
    let pause = backoff
        .pause(&timed_out)
        .expect("a stalled connection counts as overload");
    assert!(pause >= Duration::from_millis(500) && pause <= Duration::from_millis(1_500));
    let denied = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
    assert_eq!(backoff.pause(&denied), None);
    let mut impatient = Backoff::new(&progress).with_patience(Duration::ZERO);
    assert_eq!(impatient.pause(&overload(None)), None);
}

#[test]
fn transfer_engine_task_sync_overload_is_repeated_until_it_succeeds() {
    let progress = Progress::default();
    let cancel = AtomicBool::new(false);
    let attempts = Cell::new(0);
    let result = under_permits(
        &cancel,
        &progress,
        || Some(None::<FlowPermit>),
        |_| {
            attempts.set(attempts.get() + 1);
            if attempts.get() < 3 {
                Err(overload(Some(Duration::from_millis(1))))
            } else {
                Ok("listed")
            }
        },
    );
    assert_eq!(result.map(|result| result.ok()), Some(Some("listed")));
    assert_eq!(attempts.get(), 3);

    let failures = Cell::new(0);
    let failed = under_permits(
        &cancel,
        &progress,
        || Some(None::<FlowPermit>),
        |_| -> io::Result<()> {
            failures.set(failures.get() + 1);
            Err(io::Error::new(io::ErrorKind::NotFound, "gone"))
        },
    );
    assert!(matches!(failed, Some(Err(error)) if error.kind() == io::ErrorKind::NotFound));
    assert_eq!(failures.get(), 1);
}

#[test]
fn transfer_engine_task_sync_overload_wait_ends_on_cancel() {
    let progress = Progress::default();
    let cancel = AtomicBool::new(true);
    let result = under_permits(
        &cancel,
        &progress,
        || Some(None::<FlowPermit>),
        |_| -> io::Result<()> { Err(overload(Some(Duration::from_secs(30)))) },
    );
    assert!(result.is_none());
}
