//! Direct retries over real QUIC (2026-11-25): on a relay-only pair the
//! production route actor of the client keeps starting direct attempts
//! through the vendored transport (`Endpoint::retry_direct_paths`) after the
//! finite direct phase, starts one at once when the peer publishes other
//! addresses, keeps the payload on the relay without a pause, and the
//! server side never retries.
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{Connection, DirectRetryOutcome};
use iroh::Endpoint;
use tokio::sync::watch;

use super::{runtime, send_routed, Pair, FILE};
use crate::share::route_diagnostics::{DirectRetryFacts, RetryCause};
use crate::share::route_monitor;
use crate::share::route_retry_port::{AttemptFuture, DirectRetryPort, RetryCandidates};
use crate::share::route_state::{RouteClass, RoutePhase, RouteStatus};

const CLIENT: &str = "route-retry-client";
const HOST: &str = "route-retry-host";

struct TestPort {
    endpoint: Endpoint,
    peer: Mutex<Vec<SocketAddr>>,
    triggers: watch::Sender<u64>,
    outcomes: Mutex<Vec<DirectRetryOutcome>>,
}

impl TestPort {
    fn new(endpoint: &Endpoint) -> Arc<Self> {
        Arc::new(Self {
            endpoint: endpoint.clone(),
            peer: Mutex::new(Vec::new()),
            triggers: watch::channel(0).0,
            outcomes: Mutex::new(Vec::new()),
        })
    }

    /// The peer republished: other addresses and a wake-up of the retries.
    fn publish(&self, peer: Vec<SocketAddr>) {
        *self.peer.lock().unwrap() = peer;
        self.triggers.send_modify(|counter| *counter += 1);
    }

    fn outcomes(&self) -> Vec<DirectRetryOutcome> {
        self.outcomes.lock().unwrap().clone()
    }
}

impl DirectRetryPort for TestPort {
    fn candidates(&self, _remote: &str) -> RetryCandidates {
        RetryCandidates {
            local: Vec::new(),
            peer: self.peer.lock().unwrap().clone(),
        }
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.triggers.subscribe()
    }

    fn attempt<'a>(
        &'a self,
        connection: &'a Connection,
        peer: Vec<SocketAddr>,
    ) -> AttemptFuture<'a> {
        Box::pin(async move {
            let outcome = self.endpoint.retry_direct_paths(connection, peer).await;
            self.outcomes.lock().unwrap().push(outcome);
            Some(outcome)
        })
    }
}

fn retry_facts(local: &str) -> Option<DirectRetryFacts> {
    route_monitor::diagnostics(local, None)
        .into_iter()
        .find_map(|facts| facts.retry)
}

/// Waits until the route reason of `status` satisfies `wanted`.
async fn reason_until(
    status: &mut watch::Receiver<RouteStatus>,
    within: Duration,
    wanted: impl Fn(&str) -> bool,
) -> RouteStatus {
    tokio::time::timeout(within, async {
        loop {
            let current = status.borrow_and_update().clone();
            if wanted(&current.reason) {
                return current;
            }
            status.changed().await.expect("route monitor");
        }
    })
    .await
    .unwrap_or_else(|_| panic!("route reason: {:?}", status.borrow().clone()))
}

#[test]
fn route_retry_task_relay_fallback_keeps_trying_the_direct_path() {
    runtime().block_on(async {
        let relay = iroh::test_utils::run_relay_server_with(false).await;
        let (map, url, _relay) = relay.expect("private relay");
        let pair = Pair::new(&map, &url, false, false).await;
        let [client_endpoint, host_endpoint] = &pair.endpoints;
        let client_port = TestPort::new(client_endpoint);
        let host_port = TestPort::new(host_endpoint);
        let port: Arc<dyn DirectRetryPort> = client_port.clone();
        route_monitor::start_with_retry(
            &pair.client,
            CLIENT.into(),
            false,
            true,
            Vec::new(),
            Some(port),
        )
        .expect("client route monitor");
        let port: Arc<dyn DirectRetryPort> = host_port.clone();
        route_monitor::start_with_retry(
            &pair.host,
            HOST.into(),
            false,
            true,
            Vec::new(),
            Some(port),
        )
        .expect("host route monitor");
        let mut status = route_monitor::subscribe(&pair.client).expect("client route");
        // The finite direct phase ends in the marked fallback; retries arm.
        let armed = reason_until(&mut status, Duration::from_secs(90), |reason| {
            reason.starts_with("Relay als Rückfall; neuer Direktversuch in 5 s (Versuch 1); ")
        })
        .await;
        assert_eq!(armed.phase, RoutePhase::RelayFallback);
        assert_eq!(armed.class, Some(RouteClass::Relay));
        assert!(armed.application_allowed);
        let facts = retry_facts(CLIENT).expect("retry facts");
        assert_eq!(facts.attempts, 0);
        assert!(
            facts.next_attempt_in_ms.is_some_and(|wait| wait <= 5_000),
            "{facts:?}"
        );
        // The first attempt 5 s later, through the transport.
        let first = reason_until(&mut status, Duration::from_secs(10), |reason| {
            reason.starts_with("Relay als Rückfall; neuer Direktversuch in 10 s (Versuch 2); ")
        })
        .await;
        let outcomes = client_port.outcomes();
        assert_eq!(outcomes.len(), 1, "{outcomes:?}");
        assert!(
            outcomes[0].started && !outcomes[0].server_side,
            "{outcomes:?}"
        );
        assert_eq!(first.pause_epoch, armed.pause_epoch, "no pause");
        // The peer publishes other addresses: an attempt at once.
        client_port.publish(vec!["192.0.2.7:4433".parse().unwrap()]);
        reason_until(&mut status, Duration::from_secs(3), |reason| {
            reason.starts_with("Relay als Rückfall; neuer Direktversuch in 5 s (Versuch 3); ")
        })
        .await;
        let facts = retry_facts(CLIENT).expect("retry facts");
        assert_eq!(facts.attempts, 2);
        assert_eq!(facts.last_cause, Some(RetryCause::PeerAddresses));
        assert_eq!(client_port.outcomes().len(), 2);
        // The relay keeps carrying payload meanwhile.
        send_routed(&pair.client, 0, FILE)
            .await
            .expect("file in relay fallback");
        pair.ledger.assert_exactly_once(1, FILE);
        assert_eq!(status.borrow().pause_epoch, armed.pause_epoch);
        // Only the client opens paths: the server side never retries.
        let host = route_monitor::snapshot(&pair.host).expect("host route");
        assert!(!host.reason.contains("Direktversuch"), "{host:?}");
        assert!(retry_facts(HOST).is_none());
        assert!(host_port.outcomes().is_empty());
        pair.close().await;
    });
}
