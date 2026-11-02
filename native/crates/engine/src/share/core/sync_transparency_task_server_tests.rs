//! Switching a plaintext Share-server address to its encrypted form.
use super::*;

fn stored(raw: &str) -> SignalServerConfig {
    SignalServerConfig::parse_stored(raw).unwrap()
}

#[test]
fn sync_transparency_task_plaintext_addresses_map_to_wss_on_the_same_server() {
    assert_eq!(
        stored("tcp://silasweis.de:51820")
            .encrypted_alternative()
            .as_deref(),
        Some("wss://silasweis.de:51820")
    );
    // A legacy value without scheme is plaintext TCP on the default port.
    assert_eq!(
        stored("share.example").encrypted_alternative().as_deref(),
        Some("wss://share.example:51820")
    );
    assert_eq!(
        stored("ws://share.example/se")
            .encrypted_alternative()
            .as_deref(),
        Some("wss://share.example/se")
    );
    assert_eq!(
        stored("ws://share.example:8080")
            .encrypted_alternative()
            .as_deref(),
        Some("wss://share.example:8080")
    );
    assert_eq!(
        stored("tcp://[2001:db8::1]:51820")
            .encrypted_alternative()
            .as_deref(),
        Some("wss://[2001:db8::1]:51820")
    );
    // The suggestion is valid input without the plaintext permission.
    let encrypted = stored("tcp://silasweis.de:51820")
        .encrypted_alternative()
        .unwrap();
    let config = SignalServerConfig::parse_input(&encrypted, false).unwrap();
    assert_eq!(config.security(), ServerSecurity::Encrypted);
}

#[test]
fn sync_transparency_task_encrypted_or_empty_configs_need_no_switch() {
    assert!(stored("wss://share.example")
        .encrypted_alternative()
        .is_none());
    assert!(stored("").encrypted_alternative().is_none());
    assert!(stored("tcp://192.0.2.7:51820").names_ip_address());
    assert!(!stored("tcp://silasweis.de:51820").names_ip_address());
}
