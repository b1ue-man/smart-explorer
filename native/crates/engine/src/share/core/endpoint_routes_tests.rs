//! `relay_mode` and `relay_config` (qad M4): QUIC address discovery is asked
//! of a relay on its QUIC port only over TLS.
use iroh::{RelayConfig, RelayMode, RelayUrl};

use super::{relay_config, relay_mode};

/// Iroh's default QUIC port of a relay (`DEFAULT_RELAY_QUIC_PORT`).
const QAD_PORT: u16 = 7842;

fn url(text: &str) -> RelayUrl {
    text.parse().expect("relay URL")
}

fn custom(urls: &[RelayUrl]) -> iroh::RelayMap {
    match relay_mode(urls) {
        RelayMode::Custom(map) => map,
        other => panic!("a relay URL gives a custom relay mode, not {other:?}"),
    }
}

#[test]
fn relay_mode_uses_the_quic_port_for_https_only() {
    // https: the QUIC port of the default configuration.
    let secure = url("https://relay.example:51821");
    let map = custom(std::slice::from_ref(&secure));
    assert_eq!(map.len(), 1);
    let config = map.get(&secure).expect("the relay is in the map");
    assert_eq!(config.quic, RelayConfig::from(secure.clone()).quic);
    assert_eq!(config.quic.as_ref().map(|quic| quic.port), Some(QAD_PORT));
    assert_eq!(
        relay_config(secure.clone())
            .quic
            .as_ref()
            .map(|quic| quic.port),
        Some(QAD_PORT)
    );
    // http: no address discovery, the relay itself stays configured.
    let plain = url("http://127.0.0.1:51821");
    assert!(relay_config(plain.clone()).quic.is_none());
    let map = custom(std::slice::from_ref(&plain));
    assert_eq!(map.len(), 1);
    assert!(map
        .get(&plain)
        .expect("the relay is in the map")
        .quic
        .is_none());
    // Mixed: each relay by its own scheme.
    let map = custom(&[secure.clone(), plain.clone()]);
    assert_eq!(map.len(), 2);
    assert!(map.get(&secure).expect("https relay").quic.is_some());
    assert!(map.get(&plain).expect("http relay").quic.is_none());
    // No relay: relays are off.
    assert_eq!(relay_mode(&[]), RelayMode::Disabled);
    // The same URL twice is one relay.
    let map = custom(&[secure.clone(), secure.clone()]);
    assert_eq!(map.len(), 1);
    assert!(map.contains(&secure));
}
