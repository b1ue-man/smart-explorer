//! The SMB protocol name and the endpoint recognition of the connect layer. Moved
//! here from smb/core/tests.rs: it names `connect`, which lies above the transfer
//! layer (plan docs/plaene/2026-11-01-idealzustand, D13).
use crate::creds::Protocol;

#[test]
fn android_task_smb_protocol_and_endpoints_are_recognized() {
    assert_eq!(Protocol::parse("smb"), Some(Protocol::Smb));
    assert_eq!(Protocol::Smb.as_str(), "smb");
    assert_eq!(Protocol::Smb.default_port(), 445);
    assert!(Protocol::Smb.is_url());

    let endpoint = "smb://FIRMA\\anna@nas:1445/daten/x";
    assert!(crate::connect::is_remote_url(endpoint));
    let (protocol, user, host, port, path) = crate::connect::parse_remote_url(endpoint).unwrap();
    assert_eq!(protocol, Protocol::Smb);
    assert_eq!(
        (user.as_str(), host.as_str(), port),
        ("FIRMA\\anna", "nas", 1445)
    );
    assert_eq!(path, "/daten/x");
    crate::connect::validate_sync_endpoints(endpoint, "/storage/emulated/0/x").unwrap();
    assert!(
        crate::connect::validate_sync_endpoints(endpoint, "smb://FIRMA\\anna@nas:1445/daten")
            .is_err()
    );
}
