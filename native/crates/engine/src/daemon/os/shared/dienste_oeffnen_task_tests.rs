//! Desktop „Öffnen“ and „Schließen“ of a shared service (E27, E50, M26): the
//! wire names of both requests, the refusal of a device without a feed, the
//! feed rules of `auto_forward` and the IPC arm on a feed of this process, and
//! the foreign-account case that the suite crates-task starts with a second
//! account. The tests with a feed bind listeners and write the device
//! address book of the app data folder: suite part `ignored` runs them alone
//! with an isolated one.
use super::{TcpIpcReply, TcpIpcRequest};
use crate::share::auto_forward::{self, feed_fixture::Feed, MappedService};

const DEVICE: &str = "dienste-oeffnen-task-geraet";
const KEY: &str = "dienste-oeffnen-task-schluessel";

#[test]
fn dienste_oeffnen_task_wire_names_of_open_and_close() {
    let open = TcpIpcRequest::OpenService {
        device_id: DEVICE.into(),
        public_key: KEY.into(),
        port: 8080,
    };
    let value = serde_json::to_value(&open).expect("open request serializes");
    assert_eq!(value["kind"], "open_service");
    assert_eq!(value["port"], 8080);
    let parsed: TcpIpcRequest = serde_json::from_value(value).expect("open request parses");
    assert_eq!(parsed, open);

    let close = TcpIpcRequest::CloseService {
        device_id: DEVICE.into(),
        public_key: KEY.into(),
        port: 8080,
    };
    let value = serde_json::to_value(&close).expect("close request serializes");
    assert_eq!(value["kind"], "close_service");
    let parsed: TcpIpcRequest = serde_json::from_value(value).expect("close request parses");
    assert_eq!(parsed, close);
}

#[test]
fn dienste_oeffnen_task_service_reply_carries_the_mapping() {
    let reply = TcpIpcReply::Service {
        mapping: MappedService {
            host_port: 8080,
            process: Some("web".into()),
            ..MappedService::default()
        },
    };
    let value = serde_json::to_value(&reply).expect("service reply serializes");
    assert_eq!(value["kind"], "service");
    assert_eq!(value["mapping"]["host_port"], 8080);
    let parsed: TcpIpcReply = serde_json::from_value(value).expect("service reply parses");
    assert_eq!(parsed, reply);
}

#[test]
fn dienste_oeffnen_task_open_refuses_a_device_without_feed() {
    // No feed ever registered this device, so no feed names the port.
    let refused = auto_forward::open(DEVICE, KEY, 8080);
    assert_eq!(
        refused,
        Err("Gerät liefert keine Dienstliste (offline oder ohne Freigabe)".to_string())
    );
}

#[test]
fn dienste_oeffnen_task_close_of_an_unknown_service_is_false() {
    assert!(!auto_forward::close(DEVICE, KEY, 8080));
}

#[test]
#[ignore = "binds listeners and writes the device address book; Suite crates-task"]
fn dienste_oeffnen_task_closed_service_is_not_restarted_by_a_feed_update() {
    let (device, key, port) = (
        "dienste-geschlossen-geraet",
        "dienste-geschlossen-schluessel",
        47311,
    );
    let feed = Feed::new(device, key);
    feed.update(&[port]).expect("the feed is applied");
    assert!(feed.listening(port), "a listed service gets a listener");
    assert!(auto_forward::close(device, key, port));
    assert!(!feed.listening(port), "„Schließen“ ends the listener");
    // The next feed update lists the same service: it stays closed.
    feed.update(&[port]).expect("the feed is applied again");
    assert!(
        !feed.listening(port),
        "a feed update restarted a closed service"
    );
    // „Öffnen“ starts it again.
    let mapping = auto_forward::open(device, key, port).expect("the service opens");
    assert!(mapping.address.is_some(), "{mapping:?}");
    assert!(feed.listening(port));
}

#[test]
#[ignore = "binds listeners and writes the device address book; Suite crates-task"]
fn dienste_oeffnen_task_open_refuses_a_port_the_feed_does_not_name() {
    let (device, key) = ("dienste-fremdport-geraet", "dienste-fremdport-schluessel");
    let feed = Feed::new(device, key);
    feed.update(&[47312]).expect("the feed is applied");
    assert_eq!(
        auto_forward::open(device, key, 47313),
        Err("Port 47313 ist auf dem Gerät nicht freigegeben".to_string())
    );
    assert!(!feed.listening(47313));
    assert!(!auto_forward::close(device, key, 47313));
}

#[test]
#[ignore = "binds listeners and writes the device address book; Suite crates-task"]
fn dienste_oeffnen_task_open_and_close_over_ipc_change_the_mapping() {
    let (device, key, port) = ("dienste-ipc-geraet", "dienste-ipc-schluessel", 47314);
    let feed = Feed::new(device, key);
    feed.update(&[port]).expect("the feed is applied");
    let host = crate::daemon::ipc_host::ShareHost::new("5".repeat(32));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local stream");
    let mut stream = std::net::TcpStream::connect(listener.local_addr().expect("address"))
        .expect("a local stream");
    let close = TcpIpcRequest::CloseService {
        device_id: device.into(),
        public_key: key.into(),
        port,
    };
    let open = TcpIpcRequest::OpenService {
        device_id: device.into(),
        public_key: key.into(),
        port,
    };

    let closed = host.tcp_request(&mut stream, close);
    assert_eq!(closed, Ok(TcpIpcReply::Closed { closed: true }));
    assert!(
        !feed.listening(port),
        "the IPC arm „Schließen“ ends the listener"
    );

    let opened = host.tcp_request(&mut stream, open);
    let mapping = match opened {
        Ok(TcpIpcReply::Service { mapping }) => mapping,
        other => panic!("the IPC arm „Öffnen“ answers with the mapping: {other:?}"),
    };
    assert_eq!(mapping.host_port, port);
    assert!(mapping.address.is_some(), "{mapping:?}");
    assert!(feed.listening(port));

    // A port the feed does not name: the plain text of `auto_forward`.
    let refused = host.tcp_request(
        &mut stream,
        TcpIpcRequest::OpenService {
            device_id: device.into(),
            public_key: key.into(),
            port: port + 1,
        },
    );
    assert_eq!(
        refused,
        Err(format!(
            "Port {} ist auf dem Gerät nicht freigegeben",
            port + 1
        ))
    );
}

// The second account is a Linux account the suite creates (`sudo useradd`).
#[cfg(unix)]
#[path = "dienste_fremdkonto_task_tests.rs"]
mod fremdkonto;
