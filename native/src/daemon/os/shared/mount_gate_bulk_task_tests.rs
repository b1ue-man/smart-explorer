//! Deterministic admission accounting; the single remote mount task selects these.
use super::*;

fn waiting() -> Arc<Waiter> {
    Arc::new(Waiter {
        wake: Condvar::new(),
        status: AtomicU8::new(WAITING),
    })
}

#[test]
fn mount_vault_task_fifo_admission_drains_10000_without_overtaking() {
    for class in [RequestClass::Metadata, RequestClass::Transfer] {
        let mut state = GateState::default();
        let waiters: Vec<_> = (0..10_000).map(|_| waiting()).collect();
        for waiter in &waiters {
            state.queue(class).entries.push_back(waiter.clone());
            *state.waiter_count(class) += 1;
        }
        for (index, waiter) in waiters.iter().enumerate() {
            state.dispatch(1);
            assert_eq!(state.active, 1);
            assert_eq!(waiter.status(), GRANTED);
            if let Some(next) = waiters.get(index + 1) {
                assert_eq!(next.status(), WAITING);
            }
            assert_eq!(*state.waiter_count(class), waiters.len() - index - 1);
            state.release(class);
        }
        assert!(state.queue(class).entries.is_empty());
        assert_eq!(*state.waiter_count(class), 0);
        assert_eq!(state.active, 0);
    }
}

#[test]
fn mount_vault_task_metadata_burst_lets_one_transfer_through_then_resumes() {
    let mut state = GateState::default();
    let metadata: Vec<_> = (0..MAX_METADATA_PRIORITY_BURST + 2)
        .map(|_| waiting())
        .collect();
    let transfer = waiting();
    state.metadata.entries.extend(metadata.iter().cloned());
    state.metadata_waiters = metadata.len();
    state.transfers.entries.push_back(transfer.clone());
    state.transfer_waiters = 1;
    for waiter in metadata.iter().take(MAX_METADATA_PRIORITY_BURST) {
        state.dispatch(1);
        assert_eq!(waiter.status(), GRANTED);
        state.release(RequestClass::Metadata);
    }
    state.dispatch(1);
    assert_eq!(
        transfer.status(),
        GRANTED,
        "the burst bound admits the transfer"
    );
    state.release(RequestClass::Transfer);
    state.dispatch(1);
    assert_eq!(metadata[MAX_METADATA_PRIORITY_BURST].status(), GRANTED);
}

/// With several slots, waiting metadata takes the slot transfers leave free
/// even when the burst bound says a transfer is next.
#[test]
fn mount_vault_task_metadata_takes_the_slot_transfers_cannot() {
    let mut state = GateState::default();
    let transfers: Vec<_> = (0..3).map(|_| waiting()).collect();
    for waiter in &transfers {
        state.transfers.entries.push_back(waiter.clone());
        state.transfer_waiters += 1;
    }
    state.dispatch(3);
    assert_eq!(state.active, 2, "transfers keep one slot free");
    assert_eq!(transfers[2].status(), WAITING);
    let metadata = waiting();
    state.metadata.entries.push_back(metadata.clone());
    state.metadata_waiters = 1;
    state.metadata_burst = MAX_METADATA_PRIORITY_BURST;
    state.dispatch(3);
    assert_eq!(metadata.status(), GRANTED);
    assert_eq!(transfers[2].status(), WAITING);
    state.release(RequestClass::Transfer);
    state.dispatch(3);
    assert_eq!(transfers[2].status(), GRANTED);
}

#[test]
fn mount_vault_task_abort_releases_the_whole_queue() {
    let mut state = GateState::default();
    let waiters: Vec<_> = (0..128).map(|_| waiting()).collect();
    state.metadata.entries.extend(waiters.iter().cloned());
    state.metadata_waiters = waiters.len();
    for waiter in waiters.iter().take(64) {
        state.dispatch(1);
        assert_eq!(waiter.status(), GRANTED);
        state.release(RequestClass::Metadata);
    }
    state.abort_waiters();
    assert_eq!(state.metadata_waiters, 0);
    assert!(state.metadata.entries.is_empty());
    for waiter in waiters.iter().skip(64) {
        assert_eq!(waiter.status(), ABORTED);
    }
}
