//! Which relay bind may stand for a trusted reverse proxy (R-SERVER-06).

use std::net::SocketAddr;

use super::trusted_relay_proxy;
use crate::limits::SourceClassifier;

fn trusts(proxies: &str, relay: &str) -> bool {
    let classifier = SourceClassifier::parse_proxy_ips(proxies).expect("proxy list");
    let relay = relay.parse::<SocketAddr>().expect("relay address");
    trusted_relay_proxy(&classifier, relay)
}

#[test]
fn unspecified_addresses_are_not_valid_proxies() {
    for list in ["0.0.0.0", "::", "127.0.0.1, 0.0.0.0", "::1; ::"] {
        let error = SourceClassifier::parse_proxy_ips(list)
            .err()
            .unwrap_or_else(|| panic!("{list:?} was accepted"));
        assert!(error.contains("unspecified"), "{list:?}: {error}");
    }
    assert!(SourceClassifier::parse_proxy_ips("127.0.0.1, ::1").is_ok());
    assert!(SourceClassifier::parse_proxy_ips("").is_ok());
}

#[test]
fn only_a_listed_loopback_or_private_relay_bind_lifts_the_relay_source_limits() {
    let cases = [
        // (listed proxies, relay bind, the relay trusts its proxy)
        ("203.0.113.5", "203.0.113.5:51821", false),
        ("2001:db8::1", "[2001:db8::1]:51821", false),
        ("203.0.113.5", "[::ffff:203.0.113.5]:51821", false),
        ("127.0.0.1", "127.0.0.1:51821", true),
        ("::1", "[::1]:51821", true),
        ("10.0.0.2", "10.0.0.2:51821", true),
        ("172.16.0.9", "172.16.0.9:51821", true),
        ("192.168.1.2", "192.168.1.2:51821", true),
        ("fd00::1", "[fd00::1]:51821", true),
        ("10.0.0.2", "[::ffff:10.0.0.2]:51821", true),
        // The bind must be listed: any other local address stays limited.
        ("127.0.0.1", "127.0.0.2:51821", false),
        ("", "127.0.0.1:51821", false),
    ];
    for (proxies, relay, expected) in cases {
        assert_eq!(trusts(proxies, relay), expected, "{proxies:?} at {relay}");
    }
    // The limits that stay on for `false` are asserted by
    // `relay_config_applies_resource_limits` (relay.rs): 64 per source.
}
