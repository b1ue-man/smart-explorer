use crate::creds::{AuthKind, Protocol};

use super::{
    build_saved_connection, exact_match, normalize_root, read_secret_from, RemoteConnectionInput,
    MAX_STDIN_SECRET_BYTES,
};

#[test]
fn stdin_secret_is_trimmed_and_bounded() {
    assert_eq!(read_secret_from(&b"secret\r\n"[..]).unwrap(), "secret");
    let oversized = vec![b'x'; MAX_STDIN_SECRET_BYTES as usize + 1];
    assert!(read_secret_from(&oversized[..])
        .unwrap_err()
        .contains("byte limit"));
}

#[test]
fn exact_selectors_reject_missing_and_ambiguous_matches() {
    assert!(exact_match(Vec::<String>::new(), "x", "item", Clone::clone).is_err());
    assert_eq!(
        exact_match(vec!["a".to_string()], "a", "item", Clone::clone).unwrap(),
        "a"
    );
    assert!(exact_match(
        vec!["a".to_string(), "b".to_string()],
        "name",
        "item",
        Clone::clone
    )
    .unwrap_err()
    .contains("ambiguous"));
}

#[test]
fn remote_builder_applies_defaults() {
    let built = build_saved_connection(RemoteConnectionInput {
        protocol: Protocol::Sftp,
        host: Some(" example.com ".into()),
        port: None,
        user: " alice ".into(),
        root: "srv".into(),
        label: " prod ".into(),
        key: None,
        use_agent: true,
        ssh_agent: false,
        secret: Some("pw".into()),
    })
    .unwrap();
    assert_eq!(built.saved.host, "example.com");
    assert_eq!(built.saved.port, 22);
    assert_eq!(built.saved.user, "alice");
    assert_eq!(built.saved.root, "/srv");
    assert_eq!(built.saved.label, "prod");
    assert_eq!(built.saved.auth, AuthKind::Password);
    assert!(built.saved.use_agent);
    assert_eq!(built.pending_secret.as_deref(), Some("pw"));
}

#[test]
fn remote_builder_rejects_non_sftp_key_and_agent() {
    let err = build_saved_connection(RemoteConnectionInput {
        protocol: Protocol::Webdav,
        host: Some("dav.example.com".into()),
        port: None,
        user: String::new(),
        root: "/".into(),
        label: String::new(),
        key: Some("id".into()),
        use_agent: false,
        ssh_agent: false,
        secret: None,
    })
    .unwrap_err();
    assert!(err.contains("--key"));

    let err = build_saved_connection(RemoteConnectionInput {
        protocol: Protocol::Ftp,
        host: Some("ftp.example.com".into()),
        port: None,
        user: String::new(),
        root: "/".into(),
        label: String::new(),
        key: None,
        use_agent: true,
        ssh_agent: false,
        secret: None,
    })
    .unwrap_err();
    assert!(err.contains("--agent"));
}

#[test]
fn roots_are_normalized_for_remote_paths() {
    assert_eq!(normalize_root(Protocol::Sftp, "").unwrap(), "/");
    assert_eq!(normalize_root(Protocol::Sftp, "docs").unwrap(), "/docs");
    assert_eq!(
        normalize_root(Protocol::Sftp, r"\docs\team").unwrap(),
        "/docs/team"
    );
    assert_eq!(
        normalize_root(Protocol::Sftp, "/already").unwrap(),
        "/already"
    );
}

#[test]
fn share_connections_preserve_unc_roots_and_derive_host() {
    let built = build_saved_connection(RemoteConnectionInput {
        protocol: Protocol::Share,
        host: None,
        port: None,
        user: "DOMAIN\\alice".into(),
        root: "//srv/pub/team".into(),
        label: "files".into(),
        key: None,
        use_agent: false,
        ssh_agent: false,
        secret: None,
    })
    .unwrap();
    assert_eq!(built.saved.host, "srv");
    assert_eq!(built.saved.port, 0);
    assert_eq!(built.saved.root, r"\\srv\pub\team");
}

#[test]
fn luecken_task_ssh_agent_logins_store_no_secret_and_are_sftp_only() {
    let built = build_saved_connection(RemoteConnectionInput {
        protocol: Protocol::Sftp,
        host: Some("example.com".into()),
        port: None,
        user: "alice".into(),
        root: "/".into(),
        label: "agent".into(),
        key: None,
        use_agent: false,
        ssh_agent: true,
        secret: None,
    })
    .unwrap();
    assert_eq!(built.saved.auth, AuthKind::SshAgent);
    assert_eq!(built.pending_secret, None);

    let err = build_saved_connection(RemoteConnectionInput {
        protocol: Protocol::Sftp,
        host: Some("example.com".into()),
        port: None,
        user: "alice".into(),
        root: "/".into(),
        label: String::new(),
        key: Some("id_ed25519".into()),
        use_agent: false,
        ssh_agent: true,
        secret: None,
    })
    .unwrap_err();
    assert!(err.contains("--ssh-agent"));

    let err = build_saved_connection(RemoteConnectionInput {
        protocol: Protocol::Webdav,
        host: Some("dav.example.com".into()),
        port: None,
        user: String::new(),
        root: "/".into(),
        label: String::new(),
        key: None,
        use_agent: false,
        ssh_agent: true,
        secret: None,
    })
    .unwrap_err();
    assert!(err.contains("--ssh-agent"));
}
