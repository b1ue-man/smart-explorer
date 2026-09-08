//! Deterministic admission accounting; the single remote mount task selects these.
use super::*;

fn waiting(deadline: Instant, progress_timeout: Option<Duration>) -> Arc<Waiter> {
    Arc::new(Waiter { wake: Condvar::new(), status: AtomicU8::new(WAITING),
        initial_deadline: deadline, progress_timeout })
}

#[test]
fn mount_vault_task_fifo_admission_drains_10000_without_overtaking() {
    for class in [RequestClass::Metadata, RequestClass::Transfer] {
        let mut state = GateState::default();
        let deadline = Instant::now() + Duration::from_secs(60);
        let waiters: Vec<_> = (0..10_000).map(|_| waiting(deadline, None)).collect();
        for waiter in &waiters {
            state.queue(class).entries.push_back(waiter.clone());
            *state.waiter_count(class) += 1;
        }
        for (index, waiter) in waiters.iter().enumerate() {
            state.dispatch(1);
            assert_eq!(state.active, 1);
            assert_eq!(waiter.status(), GRANTED);
            if let Some(next) = waiters.get(index + 1) { assert_eq!(next.status(), WAITING); }
            assert_eq!(*state.waiter_count(class), waiters.len() - index - 1);
            state.active -= 1;
        }
        assert!(state.queue(class).entries.is_empty());
        assert_eq!(*state.waiter_count(class), 0);
        assert_eq!(state.active, 0);
    }
}

#[test]
fn mount_vault_task_metadata_progress_extends_backlog_not_transfer_deadline() {
    let start = Instant::now();
    let budget = Duration::from_secs(10);
    let metadata = waiting(start + budget, Some(budget));
    let transfer = waiting(start + budget, None);
    // Model actual service admissions, not wall-clock sleeps or arbitrary
    // notifications. A healthy queue may be much older than its idle budget.
    let mut state = GateState::default();
    for admission in 1..=10_000 {
        let served = start + Duration::from_secs(admission);
        state.reserve(RequestClass::Metadata, served);
        state.active -= 1;
        assert_eq!(metadata.deadline(state.metadata_served_at), served + budget);
        assert_eq!(transfer.deadline(state.metadata_served_at), start + budget);
    }
    let last_deadline = metadata.deadline(state.metadata_served_at);
    state.reserve(RequestClass::Transfer, start + Duration::from_secs(20_000));
    assert_eq!(metadata.deadline(state.metadata_served_at), last_deadline,
        "transfer progress must not keep an unserved metadata queue alive");
}

#[test]
fn mount_vault_task_cancelled_waiters_preserve_fifo_and_abort_releases_queue() {
    let mut state = GateState::default();
    let deadline = Instant::now() + Duration::from_secs(60);
    let waiters: Vec<_> = (0..128).map(|_| waiting(deadline, None)).collect();
    state.metadata.entries.extend(waiters.iter().cloned());
    state.metadata_waiters = waiters.len();
    for index in (1..128).step_by(2) { state.cancel(RequestClass::Metadata, &waiters[index]); }
    for index in (0..64).step_by(2) {
        state.dispatch(1);
        assert_eq!(waiters[index].status(), GRANTED);
        state.active -= 1;
    }
    state.abort_waiters();
    assert_eq!(state.metadata_waiters, 0);
    assert!(state.metadata.entries.is_empty());
    for index in (64..128).step_by(2) { assert_eq!(waiters[index].status(), ABORTED); }
    for index in (1..128).step_by(2) { assert_eq!(waiters[index].status(), CANCELED); }
}
