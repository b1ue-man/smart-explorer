use super::*;
use std::collections::HashSet;
use std::mem::MaybeUninit;
use std::sync::mpsc;

fn supervisor_without_reset_worker() -> Arc<CallbackTimeoutSupervisor> {
    // Exercise real registration/lease/schedule ownership without loading a
    // runtime or calling FFI. Only the reset-worker thread is absent. Storage
    // passed below stays allocated until all of its actual leases are finished.
    Arc::new(CallbackTimeoutSupervisor {
        shared: Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() }),
        thread: Mutex::new(None),
    })
}

#[test]
fn mount_vault_task_supervisor_registers_ten_thousand_actual_leases() {
    let supervisor = supervisor_without_reset_worker();
    assert!(matches!(supervisor.register(std::ptr::null_mut()),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput));
    // These pointers are opaque to register/finish and are never dereferenced.
    // No invented Dokany runtime function or fake reset FFI is involved.
    let mut storage = (0..10_000).map(|_| Box::new(MaybeUninit::<DokanFileInfo>::uninit()))
        .collect::<Vec<_>>();
    let mut leases = storage.iter_mut().map(|slot| {
        Some(supervisor.register(slot.as_mut_ptr()).unwrap())
    }).collect::<Vec<_>>();
    {
        let state = supervisor.shared.state.lock().unwrap();
        assert_eq!(state.requests.len(), storage.len());
        assert_eq!(state.requests.values().map(|request| request.file_info)
            .collect::<HashSet<_>>().len(), storage.len());
        assert!(state.schedule.front().is_some());
    }
    for lease in leases.iter_mut().step_by(2) { assert!(lease.take().unwrap().finish()); }
    assert_eq!(supervisor.shared.state.lock().unwrap().requests.len(), 5_000);
    // Remaining Drop paths must unlink all requests as well as explicit finish.
    drop(leases);
    let state = supervisor.shared.state.lock().unwrap();
    assert!(state.requests.is_empty());
    assert!(state.schedule.front().is_none());
    assert!(state.requests.capacity() <= 32);
    assert!(!state.failed);
}

#[test]
fn mount_vault_task_supervisor_finish_waits_for_claim_and_never_rearms() {
    let supervisor = supervisor_without_reset_worker();
    let mut storage = Box::new(MaybeUninit::<DokanFileInfo>::uninit());
    let mut lease = supervisor.register(storage.as_mut_ptr()).unwrap();
    let id = lease.id.take().unwrap();
    drop(lease);
    let request = {
        let mut state = supervisor.shared.state.lock().unwrap();
        let request = Arc::clone(&state.requests[&id]);
        assert!(state.schedule.remove(id));
        lock_request(&request).in_flight = true;
        request
    };
    let claim = ResetClaim::new(Arc::clone(&request));
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let finishing = Arc::clone(&supervisor);
    let worker = std::thread::spawn(move || {
        let lease = CallbackTimeoutLease { supervisor: &finishing, id: Some(id) };
        let _ = done_tx.send(lease.finish());
    });
    // Wait only for the observable ownership transition, not an assumed delay.
    // Release the claim before any assertion can unwind and leave finish blocked.
    let limit = Instant::now() + Duration::from_secs(5);
    let removed = loop {
        if !supervisor.shared.state.lock().unwrap().requests.contains_key(&id) { break true; }
        if Instant::now() >= limit { break false; }
        std::thread::yield_now();
    };
    let premature = done_rx.try_recv();
    let waited_for_claim = matches!(&premature, Err(mpsc::TryRecvError::Empty));
    claim.complete(true);
    let completion = match premature {
        Ok(value) => Some(value),
        Err(_) => done_rx.recv_timeout(Duration::from_secs(5)).ok(),
    };
    if completion.is_none() {
        eprintln!("callback finish did not return after claim release; aborting instead of detaching worker");
        std::process::abort();
    }
    worker.join().unwrap();
    assert!(removed, "finish must remove registration before waiting for reset");
    assert!(waited_for_claim,
        "the claimed request must outlive finish until reset completion");
    assert_eq!(completion, Some(true));
    let mut state = supervisor.shared.state.lock().unwrap();
    state.rearm_registered(id, &request, Instant::now()).unwrap();
    assert!(state.schedule.front().is_none());
    assert!(!state.requests.contains_key(&id));
}

#[test]
fn mount_vault_task_supervisor_rearm_checks_identity_stop_and_claim_failure() {
    let supervisor = supervisor_without_reset_worker();
    let mut storage = Box::new(MaybeUninit::<DokanFileInfo>::uninit());
    let lease = supervisor.register(storage.as_mut_ptr()).unwrap();
    let id = lease.id.unwrap();
    let request = {
        let mut state = supervisor.shared.state.lock().unwrap();
        let request = Arc::clone(&state.requests[&id]);
        assert!(state.schedule.remove(id));
        let different = Arc::new(Request {
            file_info: request.file_info,
            state: Mutex::new(RequestState { failed: false, in_flight: false, reported: false }),
            wake: Condvar::new(),
        });
        state.rearm_registered(id, &different, Instant::now()).unwrap();
        assert!(state.schedule.front().is_none());
        state.rearm_registered(id, &request, Instant::now()).unwrap();
        assert_eq!(state.schedule.front().unwrap().0, id);
        state.schedule.remove(id);
        state.stopped = true;
        state.rearm_registered(id, &request, Instant::now()).unwrap();
        assert!(state.schedule.front().is_none());
        state.stopped = false;
        request
    };
    lock_request(&request).in_flight = true;
    drop(ResetClaim::new(Arc::clone(&request)));
    {
        let state = lock_request(&request);
        assert!(state.failed);
        assert!(!state.in_flight);
    }
    assert!(!lease.finish());
    fail_all(&supervisor.shared);
    assert!(supervisor.failed());
    assert!(matches!(supervisor.register(storage.as_mut_ptr()),
        Err(error) if error.kind() == io::ErrorKind::Interrupted));
}
