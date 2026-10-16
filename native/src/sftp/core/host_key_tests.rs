use super::*;

#[test]
fn luecken_task_unknown_key_message_round_trips_through_its_marker() {
    let key = UnknownHostKey {
        host: "nas.example".into(),
        port: 2222,
        algorithm: "ssh-ed25519".into(),
        fingerprint: "SHA256:abcDEF123+/xyz".into(),
    };
    let message = format!("Verbindung fehlgeschlagen: {}", key.message());
    assert!(message.contains("SHA256:abcDEF123+/xyz"));
    assert!(message.contains("se connections trust-host-key nas.example:2222"));
    assert_eq!(UnknownHostKey::parse(&message), Some(key));
    assert_eq!(
        UnknownHostKey::parse("SFTP host key changed for x:22"),
        None
    );
    assert_eq!(
        UnknownHostKey::parse("[se-host-key-unknown host=x port=22 type=t fp=MD5:00]"),
        None
    );
}

#[test]
fn luecken_task_openssh_names_use_brackets_for_other_ports() {
    assert_eq!(openssh_host_name("host", 22), "host");
    assert_eq!(openssh_host_name("host", 2222), "[host]:2222");
}

#[test]
fn luecken_task_known_hosts_lines_parse_markers_and_skip_ca_lines() {
    let plain =
        parse_known_hosts_line("example.com,10.0.0.1 ssh-ed25519 AAAAC3Nz comment").unwrap();
    assert!(!plain.revoked);
    assert_eq!(plain.patterns, "example.com,10.0.0.1");
    assert_eq!(plain.key_base64, "AAAAC3Nz");
    let revoked = parse_known_hosts_line("@revoked * ssh-rsa AAAAB3Nz").unwrap();
    assert!(revoked.revoked);
    assert_eq!(
        parse_known_hosts_line("@cert-authority *.corp ssh-ed25519 AAAA"),
        None
    );
    assert_eq!(parse_known_hosts_line("# comment"), None);
    assert_eq!(parse_known_hosts_line("   "), None);
    assert_eq!(parse_known_hosts_line("only-host"), None);
}

#[test]
fn luecken_task_host_patterns_follow_openssh_wildcards_and_negation() {
    assert!(patterns_match("example.com", "example.com"));
    assert!(patterns_match("EXAMPLE.com", "example.COM"));
    assert!(patterns_match("*.example.com", "a.example.com"));
    assert!(!patterns_match("*.example.com", "example.com"));
    assert!(patterns_match("host?", "host1"));
    assert!(patterns_match("[host]:2222", "[host]:2222"));
    assert!(!patterns_match("[host]:2222", "host"));
    assert!(!patterns_match(
        "*.example.com,!bad.example.com",
        "bad.example.com"
    ));
    assert!(patterns_match(
        "*.example.com,!bad.example.com",
        "good.example.com"
    ));
    assert!(!patterns_match("!only-negated", "other"));
}

#[test]
fn luecken_task_hashed_host_names_match_like_ssh_keygen_h() {
    // `|1|salt|HMAC-SHA1(salt, name)`, computed here the way ssh-keygen -H does.
    let salt = [7u8; 20];
    let mut mac = Hmac::<Sha1>::new_from_slice(&salt).unwrap();
    mac.update(b"[nas]:2222");
    let hash = mac.finalize().into_bytes();
    let engine = base64::engine::general_purpose::STANDARD;
    let pattern = format!("|1|{}|{}", engine.encode(salt), engine.encode(hash));
    assert!(patterns_match(&pattern, "[nas]:2222"));
    assert!(!patterns_match(&pattern, "nas"));
    assert!(!patterns_match("|1|not-base64|x", "nas"));
}

#[test]
fn luecken_task_hashed_host_names_match_the_lower_case_name_like_ssh() {
    // `ssh-keygen -H` hashes the lower-case name, and `ssh` looks it up so.
    let salt = [9u8; 20];
    let mut mac = Hmac::<Sha1>::new_from_slice(&salt).unwrap();
    mac.update(b"[nas.example]:2222");
    let hash = mac.finalize().into_bytes();
    let engine = base64::engine::general_purpose::STANDARD;
    let pattern = format!("|1|{}|{}", engine.encode(salt), engine.encode(hash));
    assert!(patterns_match(&pattern, "[nas.example]:2222"));
    assert!(patterns_match(&pattern, "[NAS.Example]:2222"));
    assert!(!patterns_match(&pattern, "[nas.example]:22"));
}

#[test]
fn luecken_task_revoked_lines_keep_their_host_patterns() {
    let line = parse_known_hosts_line("@revoked !trusted.example,*.example ssh-rsa AAAAB3Nz");
    let line = line.unwrap();
    assert!(line.revoked);
    assert!(patterns_match(line.patterns, "a.example"));
    assert!(!patterns_match(line.patterns, "trusted.example"));
    assert!(!patterns_match(line.patterns, "other.org"));
    // Only the documented markers are understood; anything else is skipped.
    assert_eq!(
        parse_known_hosts_line("@unknown-marker host ssh-ed25519 AAAA"),
        None
    );
}
