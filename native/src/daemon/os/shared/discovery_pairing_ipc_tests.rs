use super::*;

fn completed(id: &str) -> DiscoveryEvent {
    DiscoveryEvent::ExchangeCompleted {
        exchange_id: id.into(),
        discovery_id: "offer".into(),
        outcome: crate::share::DiscoveryRelationOutcome::DirectInstalled {
            contact_id: "contact".into(),
            display_name: "Peer".into(),
        },
    }
}

#[test]
fn batches_task_p29_gui_drain_preserves_exact_owned_terminal_result() {
    let mut host = super::super::ipc_host::ShareHostState::new();
    let observer = host.discovery_pairing.subscribe(Some("offer".into()));
    host.discovery_pairing
        .observe(&completed("previous-exchange"));
    host.discovery_pairing.observe(&completed("ours"));
    host.ui_events
        .push(crate::share::ShareEvent::Discovery(completed("ours")));
    let _gui = std::mem::take(&mut host.ui_events);
    let result = observer
        .lock()
        .unwrap()
        .take_result("ours", "offer")
        .unwrap();
    assert!(matches!(result, Reply::Finished {
        exchange_id, status: Status::Completed, ..
    } if exchange_id == "ours"));
    assert!(host.ui_events.is_empty());
}

#[test]
fn batches_task_p29_observer_overflow_fails_closed_and_dead_observers_expire() {
    let mut observers = DiscoveryObservers::default();
    let observer = observers.subscribe(Some("offer".into()));
    for index in 0..PENDING_TERMINALS + 1 {
        observers.observe(&completed(&index.to_string()));
    }
    let state = observer.lock().unwrap();
    assert!(state.overflow);
    assert_eq!(state.terminal.len(), PENDING_TERMINALS);
    drop(state);
    drop(observer);
    observers.observe(&completed("irrelevant"));
    assert!(observers.observers.is_empty());
}

#[test]
fn batches_task_p29_connection_end_cancels_even_before_start_acknowledgement() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    server.set_read_timeout(Some(POLL)).unwrap();
    let (owner, _pending) = crate::share::DiscoveryExchangeOwner::new(None);
    let watched = ConnectionWatch::new(&server, owner.cancellation()).unwrap();
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !watched.cancel.is_cancelled() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(watched.cancel.is_cancelled());
}
