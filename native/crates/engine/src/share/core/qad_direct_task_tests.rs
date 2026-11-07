//! QUIC address discovery from the Share endpoint's side (qad M5): a relay
//! map that carries a QUIC port makes the bound endpoint probe the relay's QAD
//! service; a map without one does not. The relay is Iroh's private test
//! relay on loopback (self-signed certificate); no third-party server is
//! asked. Linux only: the relay fixture needs Iroh's `test-utils` dev feature.
//!
//! The probe's result (`Endpoint::net_report()`, `global_v4`, the direct
//! address of type `Qad`) is behind Iroh's `unstable-net-report` feature, which
//! the engine does not enable; the relay's own counter of accepted QAD
//! connections is the evidence that the probe reached the QAD service.
use std::time::{Duration, Instant};

use iroh::{Endpoint, RelayConfig, RelayMap, RelayMode, RelayUrl};

use super::{relay_config, relay_mode};

const WAIT: Duration = Duration::from_secs(20);
/// The endpoint is online (net report done) and the relay had time to count.
const GRACE: Duration = Duration::from_millis(500);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("test runtime")
}

/// The relay map production code builds for `url` (`relay_mode`), with the
/// QUIC port of the test relay, which does not listen on the default port.
fn production_map(url: &RelayUrl, served: &RelayMap) -> RelayMap {
    let RelayMode::Custom(map) = relay_mode(std::slice::from_ref(url)) else {
        panic!("a relay URL gives a custom relay mode");
    };
    let mut config: RelayConfig = (*map.get(url).expect("the relay is in the map")).clone();
    assert_eq!(config.quic, relay_config(url.clone()).quic);
    assert!(config.quic.is_some(), "an https relay asks for QAD");
    config.quic = served.get(url).expect("the test relay").quic.clone();
    assert!(config.quic.is_some(), "the test relay runs QAD");
    RelayMap::from(config)
}

/// An endpoint bound on loopback with the relay map `map`, trusting the test
/// relay's certificate, as the Share node builds its endpoint otherwise.
async fn endpoint(map: RelayMap) -> Endpoint {
    Endpoint::builder(iroh::endpoint::presets::Minimal)
        .clear_address_lookup()
        .relay_mode(RelayMode::Custom(map))
        .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
        .bind_addr("127.0.0.1:0")
        .expect("loopback address")
        .bind()
        .await
        .expect("endpoint")
}

#[test]
fn qad_direct_task_relay_with_a_quic_port_is_probed_and_one_without_is_not() {
    runtime().block_on(async {
        // With QAD: the endpoint connects to the relay's QAD service.
        let (served, url, server) = iroh::test_utils::run_relay_server_with(true)
            .await
            .expect("private relay with QAD");
        let quic_addr = server.quic_addr().expect("the relay runs the QAD service");
        assert!(quic_addr.ip().is_loopback());
        assert_ne!(quic_addr.port(), 0);
        assert_eq!(server.metrics().server.qad_connections.get(), 0);
        let probing = endpoint(production_map(&url, &served)).await;
        let started = Instant::now();
        while server.metrics().server.qad_connections.get() == 0 {
            assert!(
                started.elapsed() < WAIT,
                "the endpoint did not probe the QAD service within {WAIT:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        probing.close().await;

        // Without QAD (`RelayConfig::new(url, None)`): the endpoint reaches the
        // relay (it goes online through it) and never asks the QAD service.
        let (served, url, server) = iroh::test_utils::run_relay_server_with(true)
            .await
            .expect("second private relay with QAD");
        assert!(served.get(&url).expect("the test relay").quic.is_some());
        let silent = endpoint(RelayMap::from(RelayConfig::new(url.clone(), None))).await;
        tokio::time::timeout(WAIT, silent.online())
            .await
            .expect("the endpoint registers at the relay without QAD");
        assert!(silent.addr().relay_urls().next().is_some());
        tokio::time::sleep(GRACE).await;
        assert_eq!(server.metrics().server.qad_incoming.get(), 0);
        assert_eq!(server.metrics().server.qad_connections.get(), 0);
        silent.close().await;
    });
}
