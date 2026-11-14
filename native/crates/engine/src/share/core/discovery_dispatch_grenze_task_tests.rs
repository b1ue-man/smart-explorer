//! p-fix-c U1 (Restbeleg M34, E61): an exchange refused at the local limit of
//! active exchanges is remembered as closed before the refusal, as the other
//! refusal paths already are. A `PairingFinished` that follows for it is then
//! dropped silently: no error event and no reconnect.
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::share::discovery_exchange_port_impl::DiscoveryExchangePortImpl;
use crate::share::discovery_relation_store::InMemoryRelationStore;
use crate::share::discovery_signal_port::direct_peer_from_identity;
use crate::share::power::test_support::{auth_state, identity};
use crate::share::types::ShareAuthState;

fn discovery_runtime(auth: &Arc<Mutex<ShareAuthState>>) -> DiscoverySignalRuntime {
    let direct_peer_auth = auth.clone();
    let port = DiscoveryExchangePortImpl::new(
        Box::new(move || {
            let state = direct_peer_auth
                .lock()
                .map_err(|_| "Share-State gesperrt".to_string())?;
            direct_peer_from_identity(&state.identity)
        }),
        Box::new(InMemoryRelationStore::default()),
    );
    DiscoverySignalRuntime::with_port(Box::new(port))
}

#[test]
fn discovery_grenze_task_over_local_limit_remembers_the_refused_exchange() {
    let own = identity("grenze host").expect("identity");
    let auth = Arc::new(Mutex::new(auth_state(&own)));
    let mut runtime = discovery_runtime(&auth);
    // Pending publisher starts fill the local limit of active exchanges.
    let starts: Vec<PendingPublisherStart> = (0..MAX_ACTIVE_DISCOVERY_EXCHANGES)
        .map(|index| PendingPublisherStart {
            exchange_id: format!("pending-{index}"),
            discovery_id: "discovery-filled".into(),
            offer_id: "offer-filled".into(),
            payload: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(60),
        })
        .collect();
    runtime
        .state
        .pending_publisher_starts
        .insert("discovery-filled".into(), starts);
    assert_eq!(
        runtime.state.pending_publisher_count(),
        MAX_ACTIVE_DISCOVERY_EXCHANGES
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let client = TcpStream::connect(listener.local_addr().expect("address")).expect("client");
    // The accepted end stays alive so that the refusal can be written.
    let (_peer, _) = listener.accept().expect("peer");
    let mut signal = SignalConnection::from_test_tcp(client).expect("signal connection");
    let (events, received) = crossbeam_channel::unbounded();

    runtime
        .handle_started(
            "exchange-over".into(),
            "discovery-over".into(),
            String::new(),
            &mut signal,
            &events,
        )
        .expect("the refusal over the local limit is not an error");
    assert!(runtime.state.is_closed_exchange("exchange-over"));

    // The late PairingFinished of the refused exchange is dropped silently.
    runtime
        .handle_finished(
            "exchange-over".into(),
            PairingCloseReason::Cancelled,
            &events,
        )
        .expect("a finish of a remembered exchange is not an error");
    assert!(received
        .try_iter()
        .all(|event| !matches!(event, ShareEvent::Error(_))));
}
