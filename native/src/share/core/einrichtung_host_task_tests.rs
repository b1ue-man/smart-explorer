//! Task suite of work package WP1 „Host“ (plan 2026-10-28, design 10.1):
//! H1 local services with real listeners, H2 steering rights, and the unit
//! contracts of loopback services, TCP protocol 2, relation choice, socket
//! facts and the wire forms of design 9.2/9.4. Prefix `einrichtung_task_`.
use crate::share::access_constraints::{
    AccessRequest, NodeId, TcpDirection, TcpForwardRequest, TcpHost, TcpPort,
};
use crate::share::access_grant::{AccessGrant, Capability, CapabilityDecision};
use crate::share::loopback_services::{
    exposable, is_se_image, scope_of, services_from, BindScope, ListenerRow, LoopbackService,
    OwnerClass,
};
use crate::share::tcp_forward::{
    tcp_version_for_alpn, ListenerAdmission, TcpDestination, TcpStart, TCP_ALPN, TCP_ALPN_V1,
};
use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Mutex;

/// H1 refreshes the process-wide exposed-port set that H2 sets by hand.
static EXPOSED_PORTS: Mutex<()> = Mutex::new(());

fn tcp(port: u16, direction: TcpDirection) -> AccessRequest {
    let node = |byte: char| NodeId::parse(&byte.to_string().repeat(64)).expect("node id");
    AccessRequest::TcpForward(TcpForwardRequest {
        initiator: node('a'),
        target: node('b'),
        direction,
        host: TcpHost::Loopback,
        resolved_ip: IpAddr::from([127, 0, 0, 1]),
        port: TcpPort::try_from(port).expect("port"),
    })
}

#[test]
fn einrichtung_task_h2_steering_reaches_exactly_the_exposed_loopback_ports() {
    let _exposed = EXPOSED_PORTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::share::access_steering::note_exposed_ports([41871]);
    let mut grant = AccessGrant::default();
    grant.set(Capability::ScreenControl, CapabilityDecision::Allow);
    let exposed = tcp(41871, TcpDirection::PeerToHost);
    assert_eq!(grant.decision_for(&exposed, 0), CapabilityDecision::Allow);
    for other in [
        tcp(41872, TcpDirection::PeerToHost),
        tcp(41871, TcpDirection::HostToPeer),
    ] {
        assert_eq!(grant.decision_for(&other, 0), CapabilityDecision::Off);
    }
    grant.set(Capability::ScreenControl, CapabilityDecision::Ask);
    assert_eq!(grant.decision_for(&exposed, 0), CapabilityDecision::Off);
}

#[test]
fn einrichtung_task_binding_scope_and_exposure_rule() {
    for loopback in ["127.0.0.1", "127.5.6.7", "::1", "::ffff:127.0.0.1"] {
        let address: IpAddr = loopback.parse().expect("address");
        let scope = scope_of(address);
        assert_eq!(scope, Some(BindScope::LoopbackOnly), "{loopback}");
    }
    for wildcard in ["0.0.0.0", "::"] {
        let address: IpAddr = wildcard.parse().expect("address");
        assert_eq!(scope_of(address), Some(BindScope::Wildcard), "{wildcard}");
    }
    assert_eq!(scope_of("192.168.1.4".parse().expect("address")), None);
    let service = |scope, owner| LoopbackService {
        port: 5000,
        scope,
        owner,
        process: None,
        pid: None,
    };
    let user = OwnerClass::NormalUser {
        uid_or_sid: "1001".into(),
    };
    let none = BTreeSet::new();
    let (loopback, wildcard) = (BindScope::LoopbackOnly, BindScope::Wildcard);
    let system_loopback = service(loopback, OwnerClass::System);
    assert!(exposable(&system_loopback, &none));
    assert!(exposable(&service(wildcard, user.clone()), &none));
    assert!(!exposable(&service(wildcard, OwnerClass::System), &none));
    assert!(!exposable(&service(wildcard, OwnerClass::Unknown), &none));
    let excluded = BTreeSet::from([5000]);
    assert!(!exposable(&service(loopback, user), &excluded));
}

#[test]
fn einrichtung_task_listener_rows_fold_into_one_service_per_port() {
    let row = |local: &str, pid: Option<u32>, owner: OwnerClass| ListenerRow {
        local: local.parse::<SocketAddr>().expect("socket address"),
        pid,
        owner,
    };
    let user = OwnerClass::NormalUser {
        uid_or_sid: "1001".into(),
    };
    let rows = vec![
        row("127.0.0.1:5000", Some(10), user.clone()),
        row("0.0.0.0:5000", Some(10), user.clone()),
        row("[::1]:6000", None, OwnerClass::System),
        row("10.0.0.2:7000", Some(11), user.clone()),
        row("127.0.0.1:8000", Some(99), user.clone()),
        row("[::1]:8000", Some(100), user),
    ];
    let name = |pid: u32| (pid == 10).then(|| "python3".to_string());
    let services = services_from(&rows, |pid| pid == 99, name);
    let ports: Vec<u16> = services.iter().map(|service| service.port).collect();
    // LAN-bound 7000 and se's own 8000 are absent.
    assert_eq!(ports, vec![5000, 6000]);
    assert_eq!(services[0].scope, BindScope::Wildcard);
    assert_eq!(services[0].process.as_deref(), Some("python3"));
    assert_eq!(services[1].scope, BindScope::LoopbackOnly);
    assert_eq!(services[1].owner, OwnerClass::System);
}

#[test]
fn einrichtung_task_se_programs_next_to_the_running_exe_are_internal() {
    let exe = Path::new("/opt/se/smart_explorer");
    assert!(is_se_image(Path::new("/opt/se/se"), exe));
    let updater = Path::new("/opt/se/smart_explorer_updater");
    assert!(is_se_image(updater, exe));
    assert!(!is_se_image(Path::new("/opt/se/python3"), exe));
    assert!(!is_se_image(Path::new("/usr/bin/se"), exe));
}

#[test]
fn einrichtung_task_tcp_protocol_2_has_no_fixed_lifetime() {
    assert!(TCP_ALPN.ends_with(b"/2") && TCP_ALPN_V1.ends_with(b"/1"));
    assert_eq!(tcp_version_for_alpn(TCP_ALPN), Some(2));
    assert_eq!(tcp_version_for_alpn(TCP_ALPN_V1), Some(1));
    assert_eq!(tcp_version_for_alpn(b"smart-explorer/share-tcp/3"), None);
    let start = |lifetime_secs, acknowledge_local_access| TcpStart {
        destination: TcpDestination::from_host_port("loopback", 8080).expect("destination"),
        local_port: 0,
        acknowledge_local_access,
        lifetime_secs,
    };
    assert!(start(0, true).validate().is_ok());
    // Version 1 hosts need a fixed lifetime.
    assert!(start(0, true).validate_for(1).is_err());
    assert!(start(86_400, true).validate_for(1).is_ok());
    assert!(start(86_401, true).validate().is_err());
    assert!(start(0, false).validate().is_err());
    assert_eq!(start(0, true).expires_at(100), None);
    assert_eq!(start(0, true).fixed_lifetime(), None);
    assert_eq!(start(60, true).expires_at(100), Some(160));
    let parsed: TcpStart = serde_json::from_value(serde_json::json!({
        "destination": {"host": {"kind": "loopback"}, "port": 8080},
        "acknowledge_local_access": true
    }))
    .expect("start without lifetime");
    // An omitted lifetime means no fixed lifetime.
    assert_eq!(parsed.lifetime_secs, 0);
    assert_eq!(ListenerAdmission::default(), ListenerAdmission::SameAccount);
    assert_eq!(
        serde_json::to_value(ListenerAdmission::AwaitProcess).expect("admission"),
        serde_json::json!("await_process")
    );
}

#[test]
fn einrichtung_task_present_relation_prefers_a_present_direct_contact() {
    use crate::share::peer_select::present_relation_at;
    use crate::share::types::PeerOpenTarget;
    let now = 1_000;
    let presence = |expires_at: i64| {
        serde_json::json!({
            "kind": "direct", "relation_id": "r", "device_id": "dev", "device_name": "D",
            "public_key": "key", "fingerprint": "fp", "candidates": [],
            "expires_at": expires_at, "nonce": "n", "proof": "p"
        })
    };
    let contact = |id: &str, expires_at: i64| {
        serde_json::from_value(serde_json::json!({
            "id": id, "display_name": id, "lookup_id": id, "expected_fingerprint": "fp",
            "remote_device_id": "dev", "remote_public_key": "key",
            "auto_connect": true, "auto_open": false, "last_seen": null,
            "presence": presence(expires_at)
        }))
        .expect("contact")
    };
    let room = serde_json::from_value(serde_json::json!({
        "id": "room-profile", "name": "Room", "room_id": "room", "auto_join": true,
        "last_seen": null,
        "members": [{
            "device_id": "dev", "device_name": "D", "fingerprint": "fp", "public_key": "key",
            "candidates": [], "last_seen": null, "presence": presence(now + 600)
        }]
    }))
    .expect("room");
    let mut profiles = crate::share::profiles::ShareProfiles::default();
    profiles.rooms.push(room);
    let room_only = present_relation_at(&profiles, "dev", "key", now);
    assert!(matches!(room_only, Some(PeerOpenTarget::RoomDevice { .. })));
    profiles
        .direct_contacts
        .push(contact("b-contact", now + 100));
    profiles
        .direct_contacts
        .push(contact("a-contact", now + 100));
    profiles.direct_contacts.push(contact("stale", now - 1));
    let direct = present_relation_at(&profiles, "dev", "key", now);
    assert_eq!(
        direct,
        Some(PeerOpenTarget::Direct {
            contact_id: "a-contact".into()
        }),
        "Direct before room; equal presence goes to the smaller ID"
    );
    profiles
        .direct_contacts
        .push(contact("z-recent", now + 300));
    let recent = present_relation_at(&profiles, "dev", "key", now);
    assert_eq!(
        recent,
        Some(PeerOpenTarget::Direct {
            contact_id: "z-recent".into()
        })
    );
    let foreign = present_relation_at(&profiles, "dev", "other-key", now);
    assert_eq!(foreign, None);
}

#[test]
fn einrichtung_task_apps_wire_carries_the_new_commands_and_hides_secrets() {
    use crate::share::apps_wire::{AppsCommand, AppsResult, ManagedSecret};
    let probe = serde_json::to_value(AppsCommand::RdpProbeV2).expect("command");
    assert_eq!(probe, serde_json::json!({"kind": "rdp_probe_v2"}));
    let credential = AppsCommand::RdpCredential {
        service_id: "gnome-console".into(),
    };
    let json = serde_json::to_value(&credential).expect("command");
    assert_eq!(
        json,
        serde_json::json!({"kind": "rdp_credential", "service_id": "gnome-console"})
    );
    let result = AppsResult::RdpCredential {
        username: "se-0011aabb".into(),
        password: ManagedSecret::new("geheim-123".into()),
    };
    assert!(!format!("{result:?}").contains("geheim-123"));
    let wire = serde_json::to_string(&result).expect("result");
    assert!(wire.contains("\"password\":\"geheim-123\""));
    let back: AppsResult = serde_json::from_str(&wire).expect("result");
    let AppsResult::RdpCredential { password, .. } = back else {
        panic!("credential result expected");
    };
    assert_eq!(password.expose(), "geheim-123");
}

#[test]
fn einrichtung_task_feed_update_round_trip_and_bounds() {
    use crate::screen::core::feed_host::{decode_feed_update, encode_feed_update, FeedUpdate};
    let update = FeedUpdate {
        families: None,
        families_unavailable: Some("Wayland: nur Portalauswahl".into()),
        services: Some(vec![LoopbackService {
            port: 8000,
            scope: BindScope::LoopbackOnly,
            owner: OwnerClass::System,
            process: Some("python3".into()),
            pid: Some(42),
        }]),
        services_unavailable: None,
        rdp: None,
    };
    let bytes = encode_feed_update(&update).expect("encode");
    assert_eq!(decode_feed_update(&bytes).expect("decode"), update);
    let mut zero = update.clone();
    if let Some(services) = zero.services.as_mut() {
        services[0].port = 0;
    }
    let bytes = encode_feed_update(&zero).expect("encode");
    assert!(decode_feed_update(&bytes).is_err(), "port 0 is no service");
}

/// The connection-owner facts WP2's listener admission builds on (design
/// 2.7.1, 4.4): an in-process connection belongs to this process.
#[test]
fn einrichtung_task_connection_owner_is_this_process() {
    use crate::share::os_sockets;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
    let target = listener.local_addr().expect("listener address");
    let stream = std::net::TcpStream::connect(target).expect("connect");
    let local = stream.local_addr().expect("local address");
    let owner = os_sockets::connection_owner(local, target).expect("connection table");
    assert_eq!(owner, Some(std::process::id()));
    let account = os_sockets::connection_account(local, target).expect("connection table");
    let own_account = os_sockets::own_account().expect("own account");
    assert_eq!(account, Some(own_account));
    let own = os_sockets::process_identity(std::process::id()).expect("identity");
    assert!(own.started > 0);
    let pid = std::process::id();
    assert!(os_sockets::listener_owned_by(target.port(), pid).expect("table"));
    assert!(!os_sockets::listener_owned_by(target.port(), 1).expect("table"));
}

#[cfg(target_os = "linux")]
#[test]
fn einrichtung_task_proc_net_tcp_rows_parse() {
    use crate::share::os_sockets::{owner_class, parse_table, start_time_from_stat, uid_min_from};
    let hex = |bytes: [u8; 4]| format!("{:08X}", u32::from_ne_bytes(bytes));
    let text = format!(
        "  sl local_address rem_address st tx_queue rx_queue tr tm->when retrnsmt uid \
         timeout inode\n   0: {}:1F90 {}:0000 0A 00000000:00000000 00:00000000 00000000 \
         1001 0 4242 1\n",
        hex([127, 0, 0, 1]),
        hex([0, 0, 0, 0])
    );
    let rows = parse_table(&text);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].local, "127.0.0.1:8080".parse().expect("address"));
    assert_eq!(rows[0].state, 0x0A);
    assert_eq!(rows[0].uid, 1001);
    assert_eq!(rows[0].inode, 4242);
    assert_eq!(uid_min_from("# c\nUID_MIN\t\t 500\n"), 500);
    assert_eq!(uid_min_from(""), 1000);
    assert_eq!(owner_class(0, 1000), OwnerClass::System);
    assert_eq!(owner_class(65534, 1000), OwnerClass::System);
    let person = owner_class(1001, 1000);
    assert!(matches!(person, OwnerClass::NormalUser { .. }));
    let stat = "77 (a (b) c) S 1 77 77 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 1 0 98765 1 2";
    assert_eq!(start_time_from_stat(stat), Some(98765));
}

#[cfg(windows)]
#[test]
fn einrichtung_task_windows_owner_classes() {
    use crate::share::os_sockets::{own_account, sid_class};
    assert_eq!(sid_class("S-1-5-18"), OwnerClass::System);
    assert_eq!(sid_class("S-1-5-80-1-2-3-4-5"), OwnerClass::System);
    for person in ["S-1-5-21-1-2-3-1001", "S-1-12-1-1-2-3-4"] {
        let class = sid_class(person);
        assert!(matches!(class, OwnerClass::NormalUser { .. }), "{person}");
    }
    assert!(own_account().expect("own account").starts_with("S-1-"));
}

/// H1 (design 10.1): a Python HTTP service on 127.0.0.1:<p> and one on
/// 0.0.0.0:<q> of the test account are listed; se's own listener is not;
/// on Windows the wildcard system services 135 and 445 are not.
#[test]
fn einrichtung_task_h1_local_services_lists_user_listeners() {
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let free_port = || {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
        probe.local_addr().expect("free port").port()
    };
    let own = std::net::TcpListener::bind("127.0.0.1:0").expect("own listener");
    let own_port = own.local_addr().expect("own listener").port();
    let (p, q) = (free_port(), free_port());
    let python = if cfg!(windows) { "python" } else { "python3" };
    let spawn = |bind: &str, port: u16| {
        let child = std::process::Command::new(python)
            .args(["-m", "http.server", "--bind", bind, &port.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("python3 (python on Windows) must be installed for H1");
        Child(child)
    };
    let _loopback = spawn("127.0.0.1", p);
    let _wildcard = spawn("0.0.0.0", q);
    // Test harness bound: the interpreter starts within 20 s on CI runners.
    let ready = |port: u16| {
        (0..400).any(|_| {
            let up = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
            if !up {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            up
        })
    };
    assert!(ready(p) && ready(q), "python listeners did not come up");
    let exposed = EXPOSED_PORTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let services = crate::share::tcp_sockets::local_services().expect("local services");
    crate::share::access_steering::note_exposed_ports([]);
    drop(exposed);
    let find = |port: u16| services.iter().find(|service| service.port == port);
    let loopback = find(p).expect("loopback python service listed");
    assert_eq!(loopback.scope, BindScope::LoopbackOnly);
    let name = loopback
        .process
        .clone()
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(name.contains("python"), "process name {name:?}");
    let wildcard = find(q).expect("wildcard python service of the test account listed");
    assert_eq!(wildcard.scope, BindScope::Wildcard);
    assert!(matches!(wildcard.owner, OwnerClass::NormalUser { .. }));
    // se's own listener is never exposed.
    assert!(find(own_port).is_none());
    if cfg!(windows) {
        assert!(find(135).is_none() && find(445).is_none());
    }
    drop(own);
}
