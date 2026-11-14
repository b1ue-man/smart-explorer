//! P1 of plan `docs/plaene/2026-11-14-verbindungen-je-zweck` (section D2): remote-suite
//! oracles of the vendored QUIC patch `vendor/SMART_EXPLORER_QUIC_PROFILES.md`, written
//! against the public noq-proto and iroh API.
//!
//! The vendored crates carry the same checks as their own unit tests, but those need the
//! crates' dev-dependencies, which `native/Cargo.lock` does not resolve for patched
//! dependencies (`assert_matches`, `test-strategy`, `directories-next` are absent), so
//! `cargo test -p noq-proto` cannot build them. These tests run in the engine test binary.

use std::num::NonZeroUsize;

use noq_proto::{MtuDiscoveryConfig, TransportConfig};

fn nonzero(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("nonzero test value")
}

#[test]
fn verbindungen_task_vendor_transmit_defaults_setters_and_relay_cap() {
    let mut config = TransportConfig::default();
    assert_eq!(config.effective_max_transmit_segments().get(), 10);
    assert_eq!(config.effective_max_transmit_datagrams().get(), 20);

    config
        .max_transmit_segments(nonzero(40))
        .max_transmit_datagrams(nonzero(80));
    assert_eq!(config.effective_max_transmit_segments().get(), 40);
    assert_eq!(config.effective_max_transmit_datagrams().get(), 80);

    // 64 KiB relay packet less its 36 bytes of framing / 1452-byte segments
    // (default MTU discovery upper bound): 45 segments.
    config.max_transmit_segments(nonzero(64));
    let capped = config.effective_max_transmit_segments().get();
    assert_eq!(capped, (65_536 - 36) / 1452);
    assert!(capped * 1452 + 36 <= 65_536);
    // An upper bound at which 64 KiB / segment would pass the relay's limit
    // with the framing (45 x 1456 + 36 > 64 KiB): the cap counts the framing.
    let mut wide = MtuDiscoveryConfig::default();
    wide.upper_bound(1456);
    config.mtu_discovery_config(Some(wide));
    let capped = config.effective_max_transmit_segments().get();
    assert_eq!(capped, 44);
    assert!(capped * 1456 + 36 <= 65_536);
    // Jumbo segments: a raised value never falls below the upstream default.
    let mut jumbo = MtuDiscoveryConfig::default();
    jumbo.upper_bound(9000);
    config.mtu_discovery_config(Some(jumbo));
    assert_eq!(config.effective_max_transmit_segments().get(), 10);
}

#[test]
fn verbindungen_task_vendor_iroh_transmit_setters_reach_noq_config() {
    let config = iroh::endpoint::QuicTransportConfig::builder()
        .max_transmit_segments(nonzero(40))
        .max_transmit_datagrams(nonzero(80))
        .build();
    let text = format!("{config:?}");
    assert!(text.contains("max_transmit_segments: 40"), "{text}");
    assert!(text.contains("max_transmit_datagrams: 80"), "{text}");
    let default = format!("{:?}", iroh::endpoint::QuicTransportConfig::default());
    assert!(default.contains("max_transmit_segments: 10"), "{default}");
    assert!(default.contains("max_transmit_datagrams: 20"), "{default}");
}
