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
/// process's connection, naming the bound PID it is not (review R3: only
/// the bound process's sockets are read); a connection of this account
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
        refused.contains(&format!("nicht PID {}", owner.id())),
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
#[ignore = "remote suite: needs passwordless sudo to run a client as nobody; Suite speed-setup-task"]
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
#[ignore = "remote suite: needs the native RDP client (mstsc; xfreerdp3 with DISPLAY); Suite speed-setup-task"]
fn einrichtung_task_rdp_client_connects_from_its_own_process() {
    let listener = TcpListener::bind((task_address(), 0)).unwrap();
    let local = listener.local_addr().unwrap();
    let mut client = if cfg!(windows) {
        Command::new("mstsc.exe")
            .arg(format!("/v:{local}"))
            .spawn()
            .unwrap()
    } else {
        // FreeRDP 3 ends `+auth-only` before connecting when no password
        // is set ("auth-only, but no password set"); the test pair is fake.
        Command::new("xfreerdp3")
            .args([
                format!("/v:{local}"),
                "/cert:ignore".into(),
                "+auth-only".into(),
                "/u:se-task".into(),
                "/p:se-task".into(),
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

/// T-G1 (r-entdeckung-9, D6 of 2026-11-07): the budget of the OS adapter is a
/// quarter of the open-file limit it raised (Unix) or none (Windows).
#[test]
fn exec_task_g1_listener_budget_is_a_quarter_of_the_raised_file_limit() {
    #[cfg(windows)]
    assert_eq!(crate::share::os_sockets::automatic_listener_budget(), None);
    #[cfg(unix)]
    listener_budget::quarter_of_the_file_limit();
}

/// T-G1: two devices with 200 services each stay within the budget; above
/// it the rest has no address and the reason, `open` binds one beyond it. That
/// branch runs only where the budget is below 400 (hard open-file limit under
/// 1600): the budget is fixed per process.
#[cfg(unix)]
#[test]
fn exec_task_g1_automatic_listeners_stay_within_the_os_budget() {
    listener_budget::two_devices_within_the_budget();
}

#[cfg(unix)]
mod listener_budget {
    use crate::share::auto_forward::{
        apply, close, open, remove, remove_all, views, MappedService,
    };
    use crate::share::device_address::device_loopback;
    use crate::share::loopback_services::{BindScope, LoopbackService, OwnerClass};
    use crate::share::os_sockets::{
        automatic_listener_budget, budget_for_soft_limit, open_file_raise_target,
    };
    use crate::share::service_tests::test_service;
    use crate::share::PeerOpenTarget;
    use std::time::{Duration, Instant};

    const DEVICES: [(&str, &str); 3] = [("g1-a", "key-a"), ("g1-b", "key-b"), ("g1-c", "key-c")];

    /// Runs `body` on its own thread with the app data (the address book of
    /// the device addresses) in a new directory; fails when it runs over.
    fn isolated(limit: Duration, body: impl FnOnce() + Send + 'static) {
        let worker = std::thread::spawn(move || {
            let dir = tempfile::tempdir().expect("temporary data home");
            let previous = std::env::var_os("XDG_DATA_HOME");
            std::env::set_var("XDG_DATA_HOME", dir.path());
            let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
            match previous {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            ended
        });
        let started = Instant::now();
        while !worker.is_finished() {
            assert!(
                started.elapsed() < limit,
                "the test body ran over {limit:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Err(panic) = worker.join().expect("the test thread ended") {
            std::panic::resume_unwind(panic);
        }
    }

    fn feed(first_port: u16, count: u16) -> Vec<LoopbackService> {
        let owner = OwnerClass::NormalUser {
            uid_or_sid: "1000".into(),
        };
        (first_port..first_port + count)
            .map(|port| LoopbackService {
                port,
                scope: BindScope::LoopbackOnly,
                owner: owner.clone(),
                process: None,
                pid: None,
            })
            .collect()
    }

    fn mapped(device: usize) -> Vec<MappedService> {
        views(DEVICES[device].0, DEVICES[device].1)
    }

    fn bound(list: &[MappedService]) -> usize {
        list.iter().filter(|view| view.address.is_some()).count()
    }

    pub(super) fn two_devices_within_the_budget() {
        isolated(Duration::from_secs(120), || {
            remove_all();
            let service = test_service();
            let target = PeerOpenTarget::Direct {
                contact_id: "g1-task".into(),
            };
            let [(id_a, key_a), (id_b, key_b), (id_c, key_c)] = DEVICES;
            let limit = automatic_listener_budget().unwrap_or(usize::MAX);
            apply(&service, id_a, key_a, &target, &feed(21_000, 200)).expect("first device");
            apply(&service, id_b, key_b, &target, &feed(22_000, 200)).expect("second device");
            let (first, second) = (mapped(0), mapped(1));
            assert_eq!(first.len() + second.len(), 400);
            for (list, (id, key)) in [(&first, DEVICES[0]), (&second, DEVICES[1])] {
                let address = device_loopback(id, key).expect("device address");
                for view in list.iter().filter(|view| view.address.is_some()) {
                    assert_eq!(view.address.map(|socket| socket.ip()), Some(address.into()));
                    assert_eq!(view.last_error, None);
                }
            }
            if limit >= 400 {
                assert_eq!(bound(&first) + bound(&second), 400);
            } else {
                assert_eq!(bound(&first) + bound(&second), limit);
                let reason = format!("Nicht automatisch verbunden: höchstens {limit} Dienste aller Geräte gleichzeitig");
                for view in first.iter().chain(&second).filter(|v| v.address.is_none()) {
                    assert_eq!(view.last_error.as_deref(), Some(reason.as_str()));
                }
                let free = [(id_a, key_a, &first), (id_b, key_b, &second)]
                    .into_iter()
                    .find_map(|(id, key, list)| {
                        let view = list.iter().find(|view| view.address.is_none());
                        view.map(|view| (id, key, view.host_port))
                    });
                let (id, key, port) = free.expect("a service without an address");
                let opened = open(id, key, port).expect("open binds beyond the budget");
                assert!(opened.address.is_some());
                assert_eq!(bound(&mapped(0)) + bound(&mapped(1)), limit + 1);
                assert!(close(id, key, port));
            }
            // A gone device frees its places: the other binds all it lists.
            remove(id_a, key_a);
            assert!(mapped(0).is_empty());
            apply(&service, id_b, key_b, &target, &feed(22_000, 200)).expect("second device");
            assert_eq!(bound(&mapped(1)), 200);
            // Below the budget nothing changes: three services, three listeners.
            remove_all();
            apply(&service, id_c, key_c, &target, &feed(23_000, 3)).expect("third device");
            assert_eq!((mapped(2).len(), bound(&mapped(2))), (3, 3));
            remove_all();
        });
    }

    /// The soft and the hard open-file limit of this process (`RLIM_INFINITY`
    /// is `u64::MAX`).
    #[allow(clippy::unnecessary_cast)] // `rlim_t` is 32 bits wide on some targets
    fn limits() -> (u64, u64) {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit writes the provided live structure for this process.
        let read = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
        assert_eq!(read, 0, "getrlimit(RLIMIT_NOFILE)");
        let number = |value: libc::rlim_t| match value {
            libc::RLIM_INFINITY => u64::MAX,
            other => other as u64,
        };
        (number(limit.rlim_cur), number(limit.rlim_max))
    }

    pub(super) fn quarter_of_the_file_limit() {
        let nr_open = Some(1_048_576);
        assert_eq!(open_file_raise_target(1024, 524_288, nr_open), 524_288);
        assert_eq!(open_file_raise_target(1024, u64::MAX, nr_open), 1_048_576);
        assert_eq!(open_file_raise_target(1024, u64::MAX, None), 1024);
        assert_eq!(open_file_raise_target(1024, 2_000_000, nr_open), 1_048_576);
        assert_eq!(open_file_raise_target(u64::MAX, 100, Some(50)), u64::MAX);
        for (soft, budget) in [(0, 256), (1024, 256), (4096, 1024)] {
            assert_eq!(budget_for_soft_limit(soft), budget, "soft limit {soft}");
        }
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            budget_for_soft_limit(u64::MAX),
            usize::try_from(u64::MAX / 4).unwrap()
        );
        // The live process: the first call raises the soft limit to the hard
        // one (capped by the kernel ceiling), never lowers it.
        let (soft_before, _) = limits();
        let budget = automatic_listener_budget().expect("a budget on Unix");
        let (soft, hard) = limits();
        assert!(budget >= 256 && soft >= soft_before);
        assert_eq!(budget, budget_for_soft_limit(soft));
        let ceiling = std::fs::read_to_string("/proc/sys/fs/nr_open")
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok());
        if hard != u64::MAX && ceiling.is_none_or(|ceiling| hard <= ceiling) {
            assert_eq!(soft, hard, "the soft limit is raised to the hard limit");
        }
    }
}
