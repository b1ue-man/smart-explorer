use super::endpoint::{parse_remote_url, remote_endpoint};
use super::location::{saved_location, validate_sync_endpoints, EndpointSpec};
use crate::creds::{AuthKind, Protocol, SavedConnection};

fn connection(protocol: Protocol, root: &str) -> SavedConnection {
    SavedConnection {
        protocol, host: "example.test".into(), port: protocol.default_port(),
        user: "user@example.test".into(), root: root.into(), auth: AuthKind::Password,
        label: root.into(), use_agent: false,
    }
}

#[test]
fn sync_paths_task_picker_locators_preserve_literal_paths_and_credentials() {
    let path = "/Ordner mit Ü %20 # ?/tail \t";
    for protocol in [Protocol::Sftp, Protocol::Ftp, Protocol::Ftps, Protocol::Webdav] {
        let broad = connection(protocol, "/");
        let specific = connection(protocol, "/Ordner mit Ü %20 # ?");
        let endpoint = remote_endpoint(&specific, path);
        let EndpointSpec::Saved(parsed) = EndpointSpec::parse(&endpoint).unwrap() else {
            panic!("remote became local: {endpoint}");
        };
        let connections = [broad, specific];
        let (selected, reopened) = saved_location(&connections, &parsed).unwrap();
        assert_eq!(selected.label, "/Ordner mit Ü %20 # ?");
        assert_eq!(reopened, path);
        assert_eq!(parse_remote_url(&endpoint).unwrap().4, path);
        let mut alternate = connection(protocol, "/");
        alternate.host = "other.test".into();
        validate_sync_endpoints(&endpoint, &remote_endpoint(&alternate, path)).unwrap();
    }
    let EndpointSpec::Drive(root) = EndpointSpec::parse(&format!("gdrive://{path}")).unwrap() else { panic!() };
    assert_eq!(root, path);
    for prefix in ["share://direct/contact-a", "share://room/room-a/device-a"] {
        let EndpointSpec::Peer(target, root) = EndpointSpec::parse(&format!("{prefix}{path}")).unwrap() else { panic!() };
        assert_eq!(target.endpoint_prefix(), prefix);
        assert_eq!(root, path);
    }
}

#[test]
fn sync_paths_task_local_roots_ipv6_and_malformed_urls_never_cross_namespaces() {
    for (input, root) in [
        ("/", "/"), ("/tmp/a%20 # ? ", "/tmp/a%20 # ? "), ("C:", "C:/"),
        (r"C:\Users\A", r"C:\Users\A"), (r"\\server\share\Ü", r"\\server\share\Ü"),
        (r"\\?\C:\data", r"\\?\C:\data"), ("/tmp/sftp://literal", "/tmp/sftp://literal"),
    ] {
        let EndpointSpec::Local(actual) = EndpointSpec::parse(input).unwrap() else { panic!("{input}") };
        assert_eq!(actual, root);
    }
    for url in ["sftp://u@[::1]:2222/Docs", "sftp://u@::1:2222/Docs"] {
        let (_, user, host, port, path) = parse_remote_url(url).unwrap();
        assert_eq!((user.as_str(), host.as_str(), port, path.as_str()), ("u", "::1", 2222, "/Docs"));
    }
    assert_eq!(parse_remote_url("SFTP://u@[::1]/Docs").unwrap().3, 22);
    for input in ["webdva://host/Docs", "sftp:/host/Docs", "share://broken", "sftp://", ""] {
        assert!(EndpointSpec::parse(input).is_err(), "{input}");
    }
    for input in ["sftp://u@host:bad/Docs", "sftp://u@/Docs", "sftp://u@[::1/Docs"] {
        assert!(parse_remote_url(input).is_none(), "{input}");
    }
}

#[test]
fn sync_paths_task_reopening_local_roots_keeps_existing_baseline_identity() {
    let old_source = r"C:\old\source";
    let old_target = r"D:\old\target";
    let before = crate::bisync::pair_id_for(
        &crate::vfs::LocalBackend::new(old_source), old_source,
        &crate::vfs::LocalBackend::new(old_target), old_target);
    let (source, source_root) = crate::connect::resolve_endpoint(old_source).unwrap();
    let (target, target_root) = crate::connect::resolve_endpoint(old_target).unwrap();
    assert_eq!(source_root, old_source);
    assert_eq!(target_root, old_target);
    assert_eq!(crate::bisync::pair_id_for(&*source, &source_root, &*target, &target_root), before);
}

#[test]
fn sync_paths_task_overlap_is_scoped_to_authority_and_respects_components() {
    for (a, b) in [
        ("sftp://u@one:22/Docs", "sftp://u@two:22/Docs"),
        ("sftp://u@one:22/Docs", "ftp://u@one:21/Docs"),
        ("share://direct/a/Docs", "share://direct/b/Docs"),
        ("gdrive:///Docs", "/Docs"), ("/data", "/database"),
    ] { validate_sync_endpoints(a, b).unwrap(); }
    for (a, b) in [
        ("/data", "/data/sub"), ("/data", "/data/../data"),
        ("sftp://u@HOST/Docs", "sftp://u@host:22/Docs/sub"),
        ("gdrive:///Docs", "gdrive:///Docs/sub"),
        (r"C:\DATA", "c:/data/sub"), ("//SERVER/SHARE", "//server/share/a"),
        ("share://room/a/b/Docs", "share://room/a/b/Docs/sub"),
    ] { assert!(validate_sync_endpoints(a, b).is_err(), "{a} -> {b}"); }
}
