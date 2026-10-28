//! WP2 task tests of the viewer's TCP side (design 4.3, 4.4, 2.7.1; 10.2
//! V1–V3; assumptions A5 and A8). Prefix `einrichtung_task_`. Tests marked
//! `ignore` need a prepared runner and run only through the remote
//! task-suite entrypoint with `--ignored`.
use crate::share::auto_forward::{bind_mapping, service_url, MappedService, SocketOwnerCheck};
use crate::share::device_address::{candidate, AddressBook};
use crate::share::peer_feed::PeerFeedView;
use crate::share::tcp_forward::ListenerAdmission;
use crate::share::tcp_registry::{BoundProcess, ListenerOptions, OriginCheck};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn task_address() -> Ipv4Addr {
    candidate("einrichtung-task-device", "einrichtung-task-key", 0)
}

#[test]
fn einrichtung_task_device_address_is_stable_and_outside_127_0() {
    assert_eq!(task_address(), task_address());
    for index in 0..2000 {
        let address = candidate(&format!("device-{index}"), "key", 0);
        let [first, second, _, last] = address.octets();
        assert_eq!(first, 127);
        assert!((1..=254).contains(&second), "{address}");
        assert!((1..=254).contains(&last), "{address}");
    }
    assert_ne!(candidate("d", "k", 0), candidate("d", "k", 1));
    assert_ne!(candidate("d", "k", 0), candidate("d", "other", 0));
}

#[test]
fn einrichtung_task_device_address_collision_takes_the_next_counter() {
    let mut book = AddressBook::default();
    book.devices
        .insert("other|key".into(), candidate("device", "key", 0));
    let assigned = book.assign("device", "key").unwrap();
    assert_eq!(assigned, candidate("device", "key", 1));
    assert_eq!(
        book.assign("device", "key").unwrap(),
        assigned,
        "never reassigned"
    );
    book.validate().unwrap();
    let mut broken = AddressBook::default();
    broken
        .devices
        .insert("a|k".into(), Ipv4Addr::new(127, 0, 0, 1));
    assert!(
        broken.validate().is_err(),
        "127.0.x.x is never a device address"
    );
    let mut twice = AddressBook::default();
    twice
        .devices
        .insert("a|k".into(), Ipv4Addr::new(127, 9, 9, 9));
    twice
        .devices
        .insert("b|k".into(), Ipv4Addr::new(127, 9, 9, 9));
    assert!(twice.validate().is_err(), "one address per device");
}

/// A8: the OS binds and connects an address of 127.0.0.0/8 other than
/// 127.0.0.1 (windows-2025 and ubuntu-24.04).
#[test]
fn einrichtung_task_device_address_binds_and_connects() {
    let address = task_address();
    let listener = TcpListener::bind((address, 0)).expect("A8: bind on the device address");
    let local = listener.local_addr().unwrap();
    assert_eq!(local.ip(), IpAddr::V4(address));
    let mut client = TcpStream::connect(local).expect("A8: connect to the device address");
    let (mut server, _) = listener.accept().unwrap();
    use std::io::{Read, Write};
    client.write_all(b"se").unwrap();
    let mut buffer = [0u8; 2];
    server.read_exact(&mut buffer).unwrap();
    assert_eq!(&buffer, b"se");
}

/// V3: a taken port maps to another port on the same device address, and
/// the reason is kept for the list („Port belegt“).
#[test]
fn einrichtung_task_taken_port_falls_back_on_the_same_address() {
    let address = task_address();
    let taken = TcpListener::bind((address, 0)).unwrap();
    let port = taken.local_addr().unwrap().port();
    let (listener, note) = bind_mapping(address, port).unwrap();
    let local = listener.local_addr().unwrap();
    assert_eq!(local.ip(), IpAddr::V4(address));
    assert_ne!(local.port(), port);
    assert_eq!(note.as_deref(), Some("Port belegt"));
    drop(taken);
    let (again, note) = bind_mapping(address, port).unwrap();
    assert_eq!(
        again.local_addr().unwrap().port(),
        port,
        "a free port is kept"
    );
    assert_eq!(note, None);
}

/// V3 on Linux: a wildcard listener of the same port also counts as taken.
#[cfg(target_os = "linux")]
#[test]
fn einrichtung_task_wildcard_taken_port_falls_back() {
    let wildcard = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let port = wildcard.local_addr().unwrap().port();
    let (listener, note) = bind_mapping(task_address(), port).unwrap();
    assert_ne!(listener.local_addr().unwrap().port(), port);
    assert_eq!(note.as_deref(), Some("Port belegt"));
}

#[test]
fn einrichtung_task_service_links_follow_the_web_ports() {
    let address: SocketAddr = "127.20.30.40:8080".parse().unwrap();
    assert_eq!(
        service_url(address, 8080).as_deref(),
        Some("http://127.20.30.40:8080")
    );
    assert_eq!(
        service_url(address, 443).as_deref(),
        Some("https://127.20.30.40:8080")
    );
    assert_eq!(service_url(address, 5432), None);
    assert_eq!(
        service_url(address, 3000).as_deref(),
        Some("http://127.20.30.40:8080")
    );
}

#[test]
fn einrichtung_task_listener_options_need_loopback_and_an_origin_check() {
    let check: Arc<dyn OriginCheck> = Arc::new(SocketOwnerCheck::new().unwrap());
    let options = |admission, bind, check: Option<Arc<dyn OriginCheck>>| ListenerOptions {
        admission,
        bind,
        check,
    };
    let lan = Some(Ipv4Addr::new(192, 0, 2, 1));
    let same = ListenerAdmission::SameAccount;
    assert!(options(same, lan, Some(check.clone())).validate().is_err());
    assert!(options(same, None, None).validate().is_err());
    assert!(options(ListenerAdmission::AwaitProcess, None, None)
        .validate()
        .is_err());
    assert!(options(same, Some(task_address()), Some(check))
        .validate()
        .is_ok());
    assert!(options(ListenerAdmission::AnyLocal, None, None)
        .validate()
        .is_ok());
}

/// Snapshots of workers before 0.5.189 have no feeds; new fields default.
#[test]
fn einrichtung_task_feed_views_read_older_and_partial_json() {
    let view: PeerFeedView = serde_json::from_str("{}").unwrap();
    assert_eq!(view, PeerFeedView::default());
    let mapped: MappedService = serde_json::from_str(r#"{"host_port":8080}"#).unwrap();
    assert_eq!(mapped.host_port, 8080);
    assert_eq!(mapped.address, None);
}

fn python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

/// A process that connects to `address` and holds the connection.
fn connector(program: &str, prefix: &[&str], address: SocketAddr) -> Child {
    let script = "import socket,sys,time\n\
        s=socket.create_connection((sys.argv[1],int(sys.argv[2])))\n\
        time.sleep(60)";
    Command::new(program)
        .args(prefix)
        .args([
            "-c",
            script,
            &address.ip().to_string(),
            &address.port().to_string(),
        ])
        .spawn()
        .expect("test connector starts")
}

/// Accepts one connection within 30 s (test bound only).
fn accept(listener: &TcpListener) -> (TcpStream, SocketAddr) {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match listener.accept() {
            Ok(accepted) => return accepted,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "no connection arrived");
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => panic!("accept: {error}"),
        }
    }
}

fn bound(child: &Child) -> BoundProcess {
    let identity = crate::share::os_sockets::process_identity(child.id()).unwrap();
    BoundProcess {
        pid: child.id(),
        started: identity.started,
    }
}

/// V1 (and the mechanism behind A5): a listener bound to one started
/// process forwards that process's connection and refuses another
/// process's connection, naming its PID; a connection of this account
/// passes `SameAccount`.
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn einrichtung_task_origin_check_admits_only_the_bound_process() {
    let check = SocketOwnerCheck::new().unwrap();
    let listener = TcpListener::bind((task_address(), 0)).unwrap();
    let local = listener.local_addr().unwrap();
    let mut owner = connector(python(), &[], local);
    let (_owned, owner_peer) = accept(&listener);
    let binding = bound(&owner);
    check.process(owner_peer, local, binding).unwrap();
    check.same_account(owner_peer, local).unwrap();
    let mut stranger = connector(python(), &[], local);
    let (_foreign, stranger_peer) = accept(&listener);
    let refused = check.process(stranger_peer, local, binding).unwrap_err();
    assert!(
        refused.contains(&format!("PID {}", stranger.id())),
        "{refused}"
    );
    let restarted = BoundProcess {
        started: binding.started.wrapping_add(1),
        ..binding
    };
    assert!(check.process(owner_peer, local, restarted).is_err());
    for child in [&mut owner, &mut stranger] {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// V2 on Linux: a process of another account (`nobody`) is refused by an
/// automatic mapping's `SameAccount` rule.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "remote suite: needs passwordless sudo to run a client as nobody"]
fn einrichtung_task_other_account_is_refused() {
    let check = SocketOwnerCheck::new().unwrap();
    let listener = TcpListener::bind((task_address(), 0)).unwrap();
    let local = listener.local_addr().unwrap();
    let mut other = connector("sudo", &["-n", "-u", "nobody", "python3"], local);
    let (_socket, peer) = accept(&listener);
    let refused = check.same_account(peer, local).unwrap_err();
    assert!(refused.contains("anderen Kontos"), "{refused}");
    let _ = other.kill();
    let _ = other.wait();
}

/// A5: the native RDP client opens its RDP connection in its own process.
/// The client is pointed at a plain listener; only the first TCP
/// connection matters, the RDP handshake itself fails.
#[cfg(any(windows, target_os = "linux"))]
#[test]
#[ignore = "remote suite: needs the native RDP client (mstsc; xfreerdp3 with DISPLAY)"]
fn einrichtung_task_rdp_client_connects_from_its_own_process() {
    let listener = TcpListener::bind((task_address(), 0)).unwrap();
    let local = listener.local_addr().unwrap();
    let mut client = if cfg!(windows) {
        Command::new("mstsc.exe")
            .arg(format!("/v:{local}"))
            .spawn()
            .unwrap()
    } else {
        Command::new("xfreerdp3")
            .args([
                format!("/v:{local}"),
                "/cert:ignore".into(),
                "+auth-only".into(),
            ])
            .spawn()
            .unwrap()
    };
    let (_socket, peer) = accept(&listener);
    let owner = crate::share::os_sockets::connection_owner(peer, local).unwrap();
    assert_eq!(
        owner,
        Some(client.id()),
        "A5: connection owned by the client"
    );
    let check = SocketOwnerCheck::new().unwrap();
    check.process(peer, local, bound(&client)).unwrap();
    let _ = client.kill();
    let _ = client.wait();
}
