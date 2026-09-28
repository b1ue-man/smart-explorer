use super::*;
use crate::share::CopyPastePeerFixture;

#[test]
fn windows_remote_task_panicked_repair_clears_running_and_allows_retry() {
    let fixture = CopyPastePeerFixture::new().unwrap();
    let peer = &fixture.peer;
    let contact = peer.node.auth.lock().unwrap().direct_contacts[0].clone();
    let candidate = DirectRepairCandidate::from_accepted_contact(0, &contact,
        peer.initial_endpoint().clone(), peer.identity.clone()).unwrap();
    let key = candidate.key.clone();
    let mut coordinator = DirectReciprocalCoordinator::detached_for_task_test(0);
    coordinator.schedule(candidate).unwrap();
    let shared = coordinator.shared.clone();
    let (completed, completion) = sync_channel(1);
    let (panicked, panic_seen) = sync_channel(1);
    coordinator.worker = Some(thread::spawn(move || {
        let mut first = true;
        run_worker_with(shared, completed, |_| {
            if std::mem::take(&mut first) { panic!("injected repair failure"); }
            DirectReciprocalTransportResult::AlreadyComplete
        }, || { let _ = panicked.try_send(()); });
    }));
    panic_seen.recv_timeout(Duration::from_secs(5)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while coordinator.repair_in_flight() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!coordinator.repair_in_flight(), "panic left the daemon reload barrier stuck");
    {
        let mut state = coordinator.shared.state.lock().unwrap();
        let task = state.tasks.get_mut(&key).unwrap();
        assert!(!task.blocked);
        assert!(task.candidate.is_some());
        task.due = Some(Instant::now());
    }
    coordinator.shared.wake.notify_one();
    completion.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!coordinator.repair_in_flight());
    coordinator.request_stop();
}
