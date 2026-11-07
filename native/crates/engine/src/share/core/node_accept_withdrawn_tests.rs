//! cr-F4c (rest, E17 M21, Plan 11.5): the relay names the sender of an incoming
//! connection. An endpoint the user removed is refused before its TLS handshake
//! unless it is pinned; this test covers the step that tells the removed
//! endpoint apart from the others.
use super::*;
use crate::share::identity::code_renewal_registry as registry;

const OWN_SEED: u8 = 255;
const REMOVED_SEED: u8 = 253;
const OTHER_SEED: u8 = 254;

/// Empties the process view of one node when the test ends, also on a panic.
struct ResetView(String);

impl Drop for ResetView {
    fn drop(&mut self) {
        registry::install(&self.0, "", Vec::new(), Vec::new(), Default::default());
    }
}

#[test]
fn code_renewal_task_relay_names_a_removed_device_only_by_its_endpoint() {
    let own = iroh::SecretKey::from_bytes(&[OWN_SEED; 32])
        .public()
        .to_string();
    let removed = iroh::SecretKey::from_bytes(&[REMOVED_SEED; 32]).public();
    let other = iroh::SecretKey::from_bytes(&[OTHER_SEED; 32]).public();
    // The relay names the sender by its endpoint id; the URL is no input here.
    let relay = |endpoint_id| IncomingAddr::Relay {
        url: "https://relay.example./".parse().expect("relay url"),
        endpoint_id,
    };
    let _reset = ResetView(own.clone());

    registry::install(
        &own,
        "lookup-l23",
        Vec::new(),
        vec![removed.to_string()],
        Default::default(),
    );
    assert!(relay_names_withdrawn(&relay(removed)));
    assert!(!relay_names_withdrawn(&relay(other)));
    assert!(!relay_names_withdrawn(&IncomingAddr::Ip(
        "192.0.2.1:51820".parse().expect("address")
    )));

    // Once the view is reset, the removed endpoint is no longer named.
    registry::install(&own, "", Vec::new(), Vec::new(), Default::default());
    assert!(!relay_names_withdrawn(&relay(removed)));
}
