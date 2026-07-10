use super::*;
use std::sync::Arc;

#[test]
fn retries_a_transient_pre_commit_failure() {
    let cancel = AtomicBool::new(false);
    let mut calls = 0;
    let result = run_with_retry(2, Duration::ZERO, &cancel, || {
        calls += 1;
        if calls == 1 {
            Err(AttemptError::pre_commit(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "temporary disconnect",
            )))
        } else {
            Ok(7)
        }
    });
    assert_eq!(result.unwrap(), 7);
    assert_eq!(calls, 2);
}

#[test]
fn unsupported_and_invalid_data_are_not_retried() {
    for kind in [io::ErrorKind::Unsupported, io::ErrorKind::InvalidData] {
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let result = run_with_retry(3, Duration::ZERO, &cancel, || {
            calls += 1;
            Err::<(), _>(AttemptError::pre_commit(io::Error::new(kind, "permanent")))
        });
        assert_eq!(result.unwrap_err().into_io().kind(), kind);
        assert_eq!(calls, 1);
    }
}

#[test]
fn transient_error_after_commit_attempt_is_not_retried() {
    let cancel = AtomicBool::new(false);
    let mut calls = 0;
    let result = run_with_retry(3, Duration::ZERO, &cancel, || {
        calls += 1;
        Err::<(), _>(AttemptError::commit_attempted(io::Error::new(
            io::ErrorKind::TimedOut,
            "commit outcome is unknown",
        )))
    });
    assert_eq!(
        result.unwrap_err().into_io().kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(calls, 1);
}

#[test]
fn cancel_interrupts_retry_wait() {
    let cancel = Arc::new(AtomicBool::new(false));
    let setter = cancel.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        setter.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let mut calls = 0;
    let result = run_with_retry(3, Duration::from_secs(5), &cancel, || {
        calls += 1;
        Err::<(), _>(AttemptError::pre_commit(io::Error::new(
            io::ErrorKind::TimedOut,
            "temporary timeout",
        )))
    });
    handle.join().unwrap();

    assert_eq!(
        result.unwrap_err().into_io().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(calls, 1);
    assert!(started.elapsed() < Duration::from_secs(2));
}
