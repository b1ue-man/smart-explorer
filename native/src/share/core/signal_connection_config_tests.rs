use super::{
    migrate_stored, parse_pin, relay_url_acceptable, ServerSecurity, SignalScheme,
    SignalServerConfig,
};

const PIN_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

#[test]
fn review_task_input_without_scheme_means_tls_on_the_server_port() {
    let config = SignalServerConfig::parse_input(" Share.Example.com ", false).unwrap();
    assert_eq!(config.canonical(), "wss://share.example.com:51820");
    assert_eq!(config.security(), ServerSecurity::Encrypted);
    let endpoint = &config.endpoints()[0];
    assert_eq!(endpoint.scheme(), SignalScheme::Wss);
    assert_eq!(
        endpoint.websocket_url().as_deref(),
        Some("wss://share.example.com:51820/")
    );
    assert_eq!(
        config.relay_urls(),
        [
            "https://share.example.com:51821",
            "https://share.example.com:51820"
        ]
    );
    assert!(!config.plaintext_permitted());
}

#[test]
fn review_task_plaintext_input_needs_the_explicit_permission() {
    for plaintext in ["tcp://host:7000", "ws://host:8080/se", "http://host/se"] {
        let error = SignalServerConfig::parse_input(plaintext, false).unwrap_err();
        assert!(error.contains("unverschlüsselt"), "{plaintext}: {error}");
    }
    let tcp = SignalServerConfig::parse_input("tcp://host:7000", true).unwrap();
    assert_eq!(tcp.canonical(), "tcp://host:7000");
    assert_eq!(tcp.security(), ServerSecurity::Plaintext);
    assert!(tcp.plaintext_permitted());
    let http = SignalServerConfig::parse_input("http://host:8080/se", true).unwrap();
    assert_eq!(http.canonical(), "ws://host:8080/se");
    assert_eq!(http.relay_urls(), ["http://host:8080"]);
}

#[test]
fn review_task_mixed_input_is_rejected_because_tls_never_falls_back() {
    let error =
        SignalServerConfig::parse_input("wss://a.example/se, tcp://b.example:1", true).unwrap_err();
    assert!(error.contains("Rückfall"), "{error}");
}

#[test]
fn review_task_legacy_value_without_scheme_keeps_plaintext_tcp() {
    let config = SignalServerConfig::parse_stored("server.example:51820").unwrap();
    assert_eq!(config.endpoints()[0].scheme(), SignalScheme::Tcp);
    assert_eq!(config.canonical(), "tcp://server.example:51820");
    assert_eq!(config.security(), ServerSecurity::Plaintext);
    assert_eq!(config.relay_urls(), ["http://server.example:51821"]);
    assert_eq!(
        migrate_stored(" server.example:51820 ").as_deref(),
        Some("tcp://server.example:51820")
    );
    assert_eq!(
        migrate_stored("bare-host").as_deref(),
        Some("tcp://bare-host:51820")
    );
}

#[test]
fn review_task_stored_mixed_list_uses_only_tls_for_signaling_and_relay() {
    let stored = "wss://share.example.com/se-share, share.example.com:51820";
    let config = SignalServerConfig::parse_stored(stored).unwrap();
    let active: Vec<_> = config
        .active_endpoints()
        .into_iter()
        .map(|endpoint| endpoint.label())
        .collect();
    assert_eq!(active, ["wss://share.example.com/se-share"]);
    assert_eq!(config.ignored_plaintext(), 1);
    assert_eq!(config.relay_urls(), ["https://share.example.com"]);
    assert_eq!(config.security(), ServerSecurity::Encrypted);
    assert!(config.summary().contains("ignoriert"));
    assert_eq!(
        migrate_stored(stored).as_deref(),
        Some("wss://share.example.com/se-share, tcp://share.example.com:51820")
    );
}

#[test]
fn review_task_https_and_http_are_stored_as_websocket_schemes() {
    let config = SignalServerConfig::parse_stored("https://share.example/se-share").unwrap();
    assert_eq!(config.canonical(), "wss://share.example/se-share");
    assert_eq!(config.relay_urls(), ["https://share.example"]);
    let plain = SignalServerConfig::parse_stored("ws://h:51820").unwrap();
    // A direct ws:// server serves its relay on the next port (S18).
    assert_eq!(plain.relay_urls(), ["http://h:51821", "http://h:51820"]);
}

#[test]
fn review_task_certificate_pin_is_parsed_and_kept() {
    let with_colons = PIN_HEX
        .as_bytes()
        .chunks(2)
        .map(|pair| std::str::from_utf8(pair).unwrap().to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(":");
    let config =
        SignalServerConfig::parse_input(&format!("wss://h:8443#sha256={with_colons}"), false)
            .unwrap();
    let pin = parse_pin(PIN_HEX).unwrap();
    assert_eq!(config.pins(), [pin]);
    assert_eq!(config.endpoints()[0].pin(), Some(pin));
    assert_eq!(config.canonical(), format!("wss://h:8443#sha256={PIN_HEX}"));
    assert_eq!(
        config.endpoints()[0].websocket_url().as_deref(),
        Some("wss://h:8443/")
    );
    assert!(SignalServerConfig::parse_input(&format!("tcp://h:1#sha256={PIN_HEX}"), true).is_err());
    assert!(SignalServerConfig::parse_input("wss://h#sha256=abcd", false).is_err());
    assert!(SignalServerConfig::parse_input("wss://h#insecure", false).is_err());
    assert!(SignalServerConfig::parse_input(
        &format!("wss://h#sha256={PIN_HEX}&sha256={PIN_HEX}"),
        false
    )
    .is_err());
}

#[test]
fn review_task_ipv6_literals_need_brackets_and_keep_them_in_urls() {
    let config = SignalServerConfig::parse_input("[::1]:9000", false).unwrap();
    let endpoint = &config.endpoints()[0];
    assert_eq!(endpoint.host(), "::1");
    assert_eq!(endpoint.port(), 9000);
    assert_eq!(
        endpoint.websocket_url().as_deref(),
        Some("wss://[::1]:9000/")
    );
    assert_eq!(config.relay_urls()[0], "https://[::1]:9001");
    assert!(SignalServerConfig::parse_input("::1", false).is_err());
    assert!(SignalServerConfig::parse_input("[::1", false).is_err());
}

#[test]
fn review_task_invalid_or_unchanged_values_are_not_migrated() {
    assert_eq!(migrate_stored("wss://share.example/se-share"), None);
    assert_eq!(migrate_stored("tcp://host:1"), None);
    assert_eq!(migrate_stored(""), None);
    assert_eq!(migrate_stored("ftp://host"), None);
    assert!(SignalServerConfig::parse_stored("user@host:1").is_err());
    assert!(SignalServerConfig::parse_stored("a b").is_err());
    assert!(SignalServerConfig::parse_stored("host:0").is_err());
    assert!(SignalServerConfig::parse_stored("tcp://host:1/path").is_err());
    assert!(SignalServerConfig::parse_stored(" ; ").unwrap().is_empty());
}

#[test]
fn review_task_plain_relays_need_the_same_permission() {
    assert!(relay_url_acceptable("https://relay.example", false));
    assert!(!relay_url_acceptable("http://relay.example:51821", false));
    assert!(relay_url_acceptable("http://relay.example:51821", true));
    assert!(!relay_url_acceptable("ftp://relay.example", true));
}
