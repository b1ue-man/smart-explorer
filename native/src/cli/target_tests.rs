use crate::creds::{AuthKind, Protocol, SavedConnection};

use super::{
    endpoint_backend_keys, is_local_namespace_root, join_under, peer_rename_scope,
    saved_namespace_key, saved_rename_key, saved_shorthand_from,
};

#[test]
fn shorthand_suffix_is_under_configured_root() {
    assert_eq!(
        join_under("/home/alice", "/docs/a.txt").unwrap(),
        "/home/alice/docs/a.txt"
    );
    assert_eq!(join_under("/", "/docs/a.txt").unwrap(), "/docs/a.txt");
    assert_eq!(join_under("/", "").unwrap(), "/");
    assert_eq!(join_under("base", "leaf").unwrap(), "/base/leaf");
    assert!(join_under("/home/alice", "../escape").is_err());
    assert!(join_under("/home/alice", "docs/./file").is_err());
}

#[test]
fn namespace_roots_are_identified_without_matching_children() {
    for root in [
        "/",
        "////",
        "C:",
        "C:/",
        "//server/share",
        "\\\\server\\share\\",
    ] {
        assert!(is_local_namespace_root(root), "expected root: {root}");
    }
    for child in ["/tmp", "C:/tmp", "//server/share/dir"] {
        assert!(!is_local_namespace_root(child), "expected child: {child}");
    }
}

#[test]
fn duplicate_saved_labels_fail_with_candidates() {
    let a = conn("prod", "alice", "/srv/a");
    let b = conn("prod", "bob", "/srv/b");
    let err =
        saved_shorthand_from("prod:/logs", "@prod:/logs", vec![a.clone(), b.clone()]).unwrap_err();
    assert!(err.contains("ambiguous"));
    assert!(err.contains(&a.account()));
    assert!(err.contains(&b.account()));
}

#[test]
fn account_shorthand_can_disambiguate_duplicate_labels() {
    let a = conn("prod", "alice", "/srv/a");
    let b = conn("prod", "bob", "/srv/b");
    let spec = format!("{}:/logs", b.account());
    let (found, path) = saved_shorthand_from(&spec, &format!("@{spec}"), vec![a, b.clone()])
        .unwrap()
        .unwrap();
    assert_eq!(found.account(), b.account());
    assert_eq!(path, "/srv/b/logs");
}

#[test]
fn endpoint_backend_keys_separate_peers_but_join_each_peer_paths() {
    let docs_a = endpoint_backend_keys("share://direct/contact-a/Docs/a.txt");
    let docs_b = endpoint_backend_keys("share://direct/contact-a/Docs/b.txt");
    let other = endpoint_backend_keys("share://direct/contact-a/Other/a.txt");
    assert_eq!(docs_a.0, docs_b.0);
    assert_eq!(docs_a.1, docs_b.1);
    assert_eq!(docs_a.0, other.0);
    assert_ne!(docs_a.1, other.1);
    assert_ne!(
        endpoint_backend_keys("share://direct/contact-a/Docs"),
        endpoint_backend_keys("share://direct/contact-b/Docs")
    );
    assert_eq!(
        endpoint_backend_keys("C:/tmp/a"),
        ("local".to_string(), "local".to_string())
    );
    assert_eq!(
        endpoint_backend_keys("gdrive:///a"),
        ("gdrive".to_string(), "gdrive".to_string())
    );
    assert_eq!(
        peer_rename_scope("/Verbindungen/Production/a.txt").as_deref(),
        Some("Verbindungen/Production")
    );
    assert_eq!(peer_rename_scope("/Verbindungen"), None);
}

#[test]
fn saved_remote_roots_share_one_backend_namespace() {
    let first = conn("first", "alice", "/srv/one");
    let second = conn("second", "alice", "/srv/two");
    assert_eq!(saved_namespace_key(&first), saved_namespace_key(&second));
    assert_ne!(saved_rename_key(&first), saved_rename_key(&second));
}

fn conn(label: &str, user: &str, root: &str) -> SavedConnection {
    SavedConnection {
        protocol: Protocol::Sftp,
        host: "example.test".into(),
        port: 22,
        user: user.into(),
        auth: AuthKind::Password,
        root: root.into(),
        label: label.into(),
        use_agent: false,
    }
}
