//! Verbindungsabbrüche (Bericht 2026-11-14), Ursache: Heimrelay auf dem
//! Signalisierungsport.
//!
//! A server address without a path (`wss://host:51820`) yields two TLS relay
//! candidates on one host: the next port, where `se-share-server` serves its
//! relay, and the origin, which is the signaling port there
//! (`SignalEndpoint::relay_urls`). The server answers QUIC address discovery
//! (QAD, UDP 7842 of the host) for any URL of that host, so with QAD for both
//! candidates Iroh measured the same latency for the signaling port, took it
//! as home relay and published it, and the device was unreachable over the
//! relay until a later net report chose again. Such candidates now start
//! without QAD; only a real relay answers their HTTPS probe, and QAD goes to
//! the candidate that became the connected home relay.
use iroh::{RelayMode, RelayUrl};

use super::{deferred_qad, relay_mode};

/// Iroh's default QUIC port of a relay (`DEFAULT_RELAY_QUIC_PORT`).
const QAD_PORT: u16 = 7842;

fn url(text: &str) -> RelayUrl {
    text.parse().expect("relay URL")
}

fn custom(urls: &[RelayUrl]) -> iroh::RelayMap {
    match relay_mode(urls) {
        RelayMode::Custom(map) => map,
        other => panic!("relay URLs give a custom relay mode, not {other:?}"),
    }
}

fn qad_port(map: &iroh::RelayMap, relay: &RelayUrl) -> Option<u16> {
    map.get(relay)
        .unwrap_or_else(|| panic!("{relay} is in the relay map"))
        .quic
        .as_ref()
        .map(|quic| quic.port)
}

#[test]
fn verbindungen_task_abbruch_signalisierungsport_wird_nicht_per_qad_zum_relay() {
    // The production chain from a stored server address without a path.
    let server =
        crate::share::server_address::SignalServerConfig::parse_stored("wss://relay.example:51820")
            .expect("stored server address");
    let urls: Vec<RelayUrl> = server.relay_urls().iter().map(|text| url(text)).collect();
    let relay = url("https://relay.example:51821");
    let signaling = url("https://relay.example:51820");
    assert_eq!(urls, vec![relay.clone(), signaling.clone()]);
    let map = custom(&urls);
    assert_eq!(map.len(), 2, "both candidates stay configured");
    assert_eq!(qad_port(&map, &relay), None, "QAD waits for the home relay");
    assert_eq!(
        qad_port(&map, &signaling),
        None,
        "the signaling port gets no QAD latency"
    );
    assert_eq!(deferred_qad(&urls), urls, "both wait for the home relay");

    // A host with one TLS candidate keeps QAD from the start.
    let other = url("https://other.example:51821");
    let map = custom(&[relay.clone(), signaling.clone(), other.clone()]);
    assert_eq!(qad_port(&map, &other), Some(QAD_PORT));
    assert_eq!(qad_port(&map, &relay), None);
    assert_eq!(
        deferred_qad(&[relay.clone(), signaling.clone(), other.clone()]),
        vec![relay.clone(), signaling.clone()]
    );

    // The same URL twice is one relay and one candidate: it keeps QAD.
    let map = custom(&[relay.clone(), relay.clone()]);
    assert_eq!(map.len(), 1);
    assert_eq!(qad_port(&map, &relay), Some(QAD_PORT));
    assert!(deferred_qad(&[relay.clone(), relay.clone()]).is_empty());

    // A plain relay is never asked for QAD and shares no TLS host.
    let plain = url("http://relay.example:51822");
    let map = custom(&[plain.clone(), relay.clone()]);
    assert_eq!(qad_port(&map, &plain), None);
    assert_eq!(qad_port(&map, &relay), Some(QAD_PORT));
    assert!(deferred_qad(&[plain, relay.clone()]).is_empty());

    // IPv6 literals and letter case: one host each.
    let v6_relay = url("https://[2001:db8::1]:51821");
    let v6_signaling = url("https://[2001:db8::1]:51820");
    let map = custom(&[v6_relay.clone(), v6_signaling.clone()]);
    assert_eq!(qad_port(&map, &v6_relay), None);
    assert_eq!(qad_port(&map, &v6_signaling), None);
    let upper = url("https://RELAY.example:51820");
    assert_eq!(deferred_qad(&[relay.clone(), upper]).len(), 2);
}

/// The endpoint itself: with the relay and a port of the same host that
/// serves no relay, every endpoint goes online at the relay, publishes only
/// it, and the relay then gets QAD. Linux only: the relay fixture needs
/// Iroh's `test-utils` dev feature (as `qad_direct_task_tests.rs`).
#[cfg(target_os = "linux")]
#[test]
fn verbindungen_task_abbruch_heimrelay_ist_nie_ein_port_ohne_relay() {
    use std::time::{Duration, Instant};

    use iroh::{Endpoint, RelayConfig};

    use super::grant_qad_to_home_relay_with;

    /// Several endpoints: with QAD for both candidates each endpoint chose
    /// the port without relay with a fair chance.
    const ENDPOINTS: usize = 4;
    const WAIT: Duration = Duration::from_secs(20);
    const GRACE: Duration = Duration::from_millis(500);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let (served, relay, server) = iroh::test_utils::run_relay_server_with(true)
            .await
            .expect("private relay with QAD");
        let qad = served.get(&relay).expect("the test relay").quic.clone();
        assert!(qad.is_some(), "the test relay runs QAD");
        // A port of the same host that accepts TCP and never answers, like
        // the signaling port that speaks no relay protocol.
        let host = relay.host_str().expect("relay host").to_string();
        let silent = std::net::TcpListener::bind((host.as_str(), 0)).expect("silent port");
        let origin: RelayUrl = format!("https://{}", silent.local_addr().expect("port"))
            .parse()
            .expect("origin URL");
        assert_eq!(origin.host_str(), relay.host_str());
        let candidates = [relay.clone(), origin.clone()];

        let RelayMode::Custom(map) = relay_mode(&candidates) else {
            panic!("relay URLs give a custom relay mode");
        };
        assert!(map.get(&relay).expect("relay").quic.is_none());
        assert!(map.get(&origin).expect("origin").quic.is_none());
        assert_eq!(server.metrics().server.qad_connections.get(), 0);

        let mut endpoints = Vec::new();
        for _ in 0..ENDPOINTS {
            // Each endpoint its own map, as each production endpoint.
            let endpoint = Endpoint::builder(iroh::endpoint::presets::Minimal)
                .clear_address_lookup()
                .relay_mode(relay_mode(&candidates))
                .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
                .bind_addr("127.0.0.1:0")
                .expect("loopback address")
                .bind()
                .await
                .expect("endpoint");
            // As production, with the QUIC port of the test relay.
            let with_qad = {
                let qad = qad.clone();
                move |url: RelayUrl| RelayConfig::new(url, qad.clone())
            };
            tokio::spawn(grant_qad_to_home_relay_with(
                endpoint.clone(),
                deferred_qad(&candidates),
                with_qad,
            ));
            endpoints.push(endpoint);
        }
        let published = |endpoint: &Endpoint| -> Vec<String> {
            endpoint
                .addr()
                .relay_urls()
                .map(ToString::to_string)
                .collect()
        };
        for (index, endpoint) in endpoints.iter().enumerate() {
            tokio::time::timeout(WAIT, endpoint.online())
                .await
                .unwrap_or_else(|_| {
                    panic!("endpoint {index} reached no home relay within {WAIT:?}")
                });
            assert_eq!(
                published(endpoint),
                vec![relay.to_string()],
                "endpoint {index} publishes only the relay"
            );
        }

        // The home relay got QAD: the relay's QAD service is asked.
        let started = Instant::now();
        while server.metrics().server.qad_connections.get() == 0 {
            assert!(
                started.elapsed() < WAIT,
                "no endpoint asked the relay's QAD service within {WAIT:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(GRACE).await;
        for (index, endpoint) in endpoints.iter().enumerate() {
            assert_eq!(
                published(endpoint),
                vec![relay.to_string()],
                "endpoint {index} keeps the relay once QAD runs"
            );
        }
        for endpoint in endpoints {
            endpoint.close().await;
        }
        drop(silent);
    });
}
