//! The login transport never uses the number0 relays (E14, r-lokal-7; suite
//! expectations E14-a to E14-e and E14-g of the review 2026-11-07): the
//! relays of the configured Share server, or none, and the explicit ones of a
//! gateway's network file.
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use iroh::{RelayUrl, SecretKey};

use super::*;
use crate::login::core::model::VERSION;

/// 00112233445566778899aabbccddeeff, twice.
const PIN_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const RELAY_ENVIRONMENT: [&str; 2] = ["SE_SHARE_RELAY_URL", "SE_SHARE_RELAY_ONLY"];

static ENVIRONMENT: Mutex<()> = Mutex::new(());

/// The relay overrides of the process removed for one test, put back after it.
struct NoRelayOverride {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl NoRelayOverride {
    fn new() -> Self {
        let lock = ENVIRONMENT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved = RELAY_ENVIRONMENT
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        for key in RELAY_ENVIRONMENT {
            std::env::remove_var(key);
        }
        Self { _lock: lock, saved }
    }
}

impl Drop for NoRelayOverride {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn relay(text: &str) -> RelayUrl {
    text.parse().expect("a relay URL")
}

fn pin() -> [u8; 32] {
    std::array::from_fn(|index| ((index % 16) * 17) as u8)
}

fn network(use_default_relays: bool, relay_urls: &[&str]) -> GatewayNetwork {
    GatewayNetwork {
        format: VERSION,
        use_default_relays,
        relay_urls: relay_urls.iter().map(ToString::to_string).collect(),
        relay_pins: Vec::new(),
        allow_plaintext_relays: false,
    }
}

fn target(use_default_relays: bool, relay_urls: &[&str]) -> GatewayTarget {
    GatewayTarget {
        format: VERSION,
        machine_id: "machine".into(),
        public_key: SecretKey::from_bytes(&[3; 32]).public().to_string(),
        direct_addresses: Vec::new(),
        relay_urls: relay_urls.iter().map(ToString::to_string).collect(),
        use_default_relays,
        allow_plaintext_relays: false,
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime")
}

/// E14-a and E14-b: "the relays of the configured server" are explicit
/// relays, pins and the plaintext rule of the stored address; no server, no
/// relays; a plaintext server keeps its plaintext rule.
#[test]
fn lokal_task_configured_server_network_is_explicit_and_never_default() {
    let _environment = NoRelayOverride::new();
    let with_server = configured_server_network(&format!(
        "wss://share.example.test/se-share#sha256={PIN_HEX}"
    ));
    assert!(!with_server.use_default_relays);
    assert_eq!(with_server.format, VERSION);
    assert_eq!(with_server.relay_urls.len(), 1);
    assert_eq!(
        relay(&with_server.relay_urls[0]),
        relay("https://share.example.test")
    );
    assert_eq!(with_server.relay_pins, vec![pin()]);
    assert!(!with_server.allow_plaintext_relays);
    with_server
        .validate()
        .expect("the resolved network is a valid network file");

    for none in ["", "   "] {
        let nothing = configured_server_network(none);
        assert!(!nothing.use_default_relays, "{none:?}");
        assert!(nothing.relay_urls.is_empty(), "{none:?}");
        assert!(nothing.relay_pins.is_empty(), "{none:?}");
        assert!(!nothing.allow_plaintext_relays, "{none:?}");
        nothing.validate().expect("an empty network is valid");
    }

    let plaintext = configured_server_network("tcp://192.0.2.7:51820");
    assert!(!plaintext.use_default_relays);
    assert!(plaintext.allow_plaintext_relays);
    assert_eq!(plaintext.relay_urls.len(), 1);
    assert_eq!(
        relay(&plaintext.relay_urls[0]),
        relay("http://192.0.2.7:51821")
    );
    plaintext.validate().expect("a plaintext network is valid");
}

/// E14-c: a gateway that stores "default relays" binds none; `bind_machine`
/// gives its endpoint no relay.
#[test]
fn lokal_task_gateway_with_default_relays_binds_no_relay() {
    let default = network(true, &[]);
    let relays = gateway_relays(&default).expect("relays of the default network");
    assert!(relays.is_empty());
    assert!(matches!(
        super::super::endpoint_routes::relay_mode(&relays),
        iroh::RelayMode::Disabled
    ));

    // A stored explicit relay is the gateway's relay.
    let explicit = network(false, &["https://gateway-relay.example.test"]);
    let relays = gateway_relays(&explicit).expect("relays of the explicit network");
    assert_eq!(relays, vec![relay("https://gateway-relay.example.test")]);

    // The endpoint of the default network has no relay and says so in its
    // address (the published target names none, E14-e).
    let runtime = runtime();
    let endpoint = runtime
        .block_on(bind_machine(SecretKey::from_bytes(&[41; 32]), &default))
        .expect("the machine endpoint binds");
    assert_eq!(endpoint.addr().relay_urls().count(), 0);
    let published = address("machine", &endpoint, &default);
    assert!(published.relay_urls.is_empty());
    assert!(!published.use_default_relays);
    assert_eq!(published.public_key, endpoint.id().to_string());

    // E14-e: explicit relays reach the published target unchanged.
    let published = address("machine", &endpoint, &explicit);
    assert_eq!(
        published.relay_urls,
        vec!["https://gateway-relay.example.test".to_string()]
    );
    assert!(!published.use_default_relays);
    assert!(!published.allow_plaintext_relays);
    drop(endpoint);
    runtime.shutdown_timeout(Duration::from_secs(5));
}

/// E14-d: a target that asks for "default relays" (also an older one) is
/// reached through the relays of the caller's configured server, never a
/// number0 relay; an explicit target keeps its own relays.
#[test]
fn lokal_task_target_with_default_relays_uses_the_callers_server_relays() {
    let configured = relay("https://configured.example.test");
    let wanted = target_address(&target(true, &[]), std::slice::from_ref(&configured))
        .expect("an address for the default target");
    assert_eq!(
        wanted.relay_urls().cloned().collect::<Vec<_>>(),
        [configured.clone()]
    );
    for url in wanted.relay_urls() {
        let text = url.to_string();
        assert!(
            !text.contains("iroh.link") && !text.contains("iroh.network") && !text.contains("n0"),
            "a number0 relay: {text}"
        );
    }

    // No configured server: only direct paths.
    let none = target_address(&target(true, &[]), &[]).expect("an address without relays");
    assert_eq!(none.relay_urls().count(), 0);

    // An explicit target keeps its relays; the caller's server is not added.
    let explicit = target_address(
        &target(false, &["https://explicit.example.test"]),
        std::slice::from_ref(&configured),
    )
    .expect("an address for the explicit target");
    assert_eq!(
        explicit.relay_urls().cloned().collect::<Vec<_>>(),
        [relay("https://explicit.example.test")]
    );

    // The direct addresses of the target stay.
    let mut with_direct = target(true, &[]);
    with_direct.direct_addresses = vec!["192.0.2.9:4433".parse().expect("an address")];
    let both = target_address(&with_direct, std::slice::from_ref(&configured))
        .expect("an address with a direct path");
    assert_eq!(both.ip_addrs().count(), 1);
    assert_eq!(both.relay_urls().count(), 1);
}

/// E14-g: a `gateway-network.json` written before the change, with
/// `use_default_relays: true`, is still read, validates, and gives the
/// gateway no relay; its field names are the ones provisioning writes.
#[test]
fn lokal_task_stored_default_relays_network_file_is_still_read() {
    let _environment = NoRelayOverride::new();
    let stored = format!(
        "{{\"format\":{VERSION},\"use_default_relays\":true,\"relay_urls\":[],\"relay_pins\":[],\"allow_plaintext_relays\":false}}"
    );
    let parsed: GatewayNetwork = serde_json::from_str(&stored).expect("the stored file parses");
    parsed.validate().expect("the stored file validates");
    assert!(parsed.use_default_relays);
    assert!(gateway_relays(&parsed)
        .expect("relays of the stored file")
        .is_empty());
    assert_eq!(parsed, GatewayNetwork::default());

    // The resolved form is written with the same field names.
    let resolved = configured_server_network(&format!("wss://share.example.test#sha256={PIN_HEX}"));
    let written: serde_json::Value = serde_json::to_value(&resolved).expect("the file form");
    let mut names: Vec<&str> = written
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "allow_plaintext_relays",
            "format",
            "relay_pins",
            "relay_urls",
            "use_default_relays"
        ]
    );
    let again: GatewayNetwork =
        serde_json::from_value(written).expect("the written file is read again");
    assert_eq!(again, resolved);
}
