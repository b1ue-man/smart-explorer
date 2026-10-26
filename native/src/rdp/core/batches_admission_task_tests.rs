//! B7: admission uses the production inventory and authenticated-probe wrapper.
use super::*;

fn service() -> HostService {
    HostService { id: "rds".into(), kind: HostKind::WindowsServer, version: None,
        session_mode: SessionMode::WindowsServerSession, port: Some(3389), running: true,
        tls: Some(true), nla: Some(true),
        certificate_sha256: Some(CertificateFingerprint::parse(&"ab".repeat(32)).unwrap()),
        certificate_source: Some("host-listener-certificate".into()),
        remote_apps: vec![PublishedApp { alias: "editor".into(), display_name: "Editor".into(), collection: "staff".into() }],
        evidence: vec!["listener owned by service".into()], unavailable_reason: None }
}

fn report(service: HostService) -> HostProbe {
    HostProbe { version: RDP_PROBE_VERSION, platform: "windows".into(), services: vec![service],
        unavailable_reason: None, diagnostics: Vec::new() }
}

fn request() -> StartRequest {
    StartRequest { service_id: "rds".into(), client_id: "freerdp3".into(),
        certificate_policy: CertificatePolicy::PeerPinOrNativeTrust, remote_app: None,
        acknowledge_local_access: true, acknowledge_session_effect: true,
        acknowledge_certificate_dialog: false, lifetime_secs: 900 }
}

#[test]
fn batches_rdp_requires_explicit_session_local_access_and_certificate_policy() {
    let probe = AuthenticatedProbe::from_authenticated_peer("host-node".into(), report(service())).unwrap();
    assert!(request().validate(&probe).is_ok());
    for case in 0..5 {
        let mut request = request();
        match case {
            0 => request.acknowledge_session_effect = false,
            1 => request.acknowledge_local_access = false,
            2 => request.certificate_policy = CertificatePolicy::ExactPeerPin,
            3 => request.certificate_policy = CertificatePolicy::NativeDialog,
            4 => request.service_id = "unadvertised-service".into(),
            _ => unreachable!(),
        }
        assert!(request.validate(&probe).is_err(), "case {case}");
    }
    let mut native = request();
    native.certificate_policy = CertificatePolicy::NativeDialog;
    native.acknowledge_certificate_dialog = true;
    assert!(native.validate(&probe).is_ok());
}

#[test]
fn batches_rdp_missing_listener_tls_certificate_or_known_session_fails_closed() {
    for case in 0..6 {
        let mut service = service();
        match case {
            0 => service.running = false,
            1 => service.port = None,
            2 => service.tls = None,
            3 => service.certificate_sha256 = None,
            4 => service.unavailable_reason = Some("service disabled by administrator".into()),
            5 => { service.kind = HostKind::Gnome; service.session_mode = SessionMode::Unknown; service.remote_apps.clear(); },
            _ => unreachable!(),
        }
        let probe = AuthenticatedProbe::from_authenticated_peer("host-node".into(), report(service)).unwrap();
        assert!(request().validate(&probe).is_err(), "case {case}");
    }
}

#[test]
fn batches_rdp_remoteapp_requires_published_server_alias_and_unambiguous_inventory() {
    let probe = AuthenticatedProbe::from_authenticated_peer("host-node".into(), report(service())).unwrap();
    let mut request = request();
    request.remote_app = Some("editor".into());
    assert!(request.validate(&probe).is_ok());
    for alias in ["not-published", "editor /shell", "../editor", ""] {
        request.remote_app = Some(alias.into());
        assert!(request.validate(&probe).is_err(), "alias {alias:?}");
    }
    let mut workstation = service();
    workstation.kind = HostKind::WindowsWorkstation;
    workstation.session_mode = SessionMode::WindowsConsoleLocked;
    assert!(AuthenticatedProbe::from_authenticated_peer("host-node".into(), report(workstation)).is_err());
    let mut duplicate = report(service());
    duplicate.services.push(service());
    assert!(AuthenticatedProbe::from_authenticated_peer("host-node".into(), duplicate).is_err());
    let mut no_provenance = service();
    no_provenance.certificate_source = None;
    assert!(AuthenticatedProbe::from_authenticated_peer("host-node".into(), report(no_provenance)).is_err());
    assert!(AuthenticatedProbe::from_authenticated_peer(String::new(), report(service())).is_err());
}

#[test]
fn batches_rdp_tunnel_is_bound_to_authenticated_peer_exact_service_and_loopback_listener() {
    use crate::share::{access_constraints::TcpDirection,
        tcp_forward::{TcpDestination, TunnelId, TunnelRole, TunnelView}};
    let probe = AuthenticatedProbe::from_authenticated_peer("host-node".into(), report(service())).unwrap();
    let service = request().validate(&probe).unwrap();
    let tunnel = TunnelView {
        id: TunnelId::parse(&"ab".repeat(16)).unwrap(), role: TunnelRole::Listener,
        remote_node: "host-node".into(), initiator_node: "viewer-node".into(), target_node: "host-node".into(),
        direction: TcpDirection::PeerToHost, relation_kind: "direct".into(), relation_id: "approved-device".into(),
        destination: TcpDestination::from_host_port("loopback", 3389).unwrap(),
        bound_target: None, listener: Some("127.0.0.1:40001".parse().unwrap()),
        started_at: 100, expires_at: 1000, active_connections: 0,
        socket_read_bytes: 0, socket_written_bytes: 0, closing: false, last_error: None,
    };
    assert!(probe.check_tunnel(service, &tunnel).is_ok());
    for case in 0..9 {
        let mut changed = tunnel.clone();
        match case {
            0 => changed.remote_node = "foreign-node".into(),
            1 => changed.target_node = "foreign-node".into(),
            2 => changed.destination = TcpDestination::from_host_port("loopback", 3390).unwrap(),
            3 => changed.destination = TcpDestination::from_host_port("192.0.2.1", 3389).unwrap(),
            4 => changed.role = TunnelRole::Target,
            5 => changed.listener = None,
            6 => changed.listener = Some("0.0.0.0:40001".parse().unwrap()),
            7 => changed.listener = Some("127.0.0.1:0".parse().unwrap()),
            8 => changed.closing = true,
            _ => unreachable!(),
        }
        assert!(probe.check_tunnel(service, &changed).is_err(), "case {case}");
    }
    let mut ipv6 = tunnel;
    ipv6.listener = Some("[::1]:40001".parse().unwrap());
    assert!(probe.check_tunnel(service, &ipv6).is_ok());
}
