use super::transport::{AgentConnection, AgentReconnect, AgentStreams, HeartbeatPolicy};
use super::AgentBackend;
use crate::agent_proto::{read_frame, write_frame, Frame, PROTO_VERSION};
use crate::vfs::{Backend, BackendHandle, LocalBackend};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn streams_to(address: SocketAddr) -> io::Result<AgentStreams> {
    let stream = TcpStream::connect(address)?;
    Ok((Box::new(stream.try_clone()?), Box::new(stream)))
}

fn handshake(socket: &mut TcpStream) -> TcpStream {
    let mut reader = socket.try_clone().unwrap();
    let (id, request) = read_frame(&mut reader).unwrap().unwrap();
    assert!(matches!(request, Frame::Hello { .. }));
    write_frame(
        socket,
        id,
        &Frame::HelloOk {
            proto: PROTO_VERSION,
            version: "test".into(),
        },
    )
    .unwrap();
    reader
}

fn backend(
    address: SocketAddr,
    reconnects: Arc<AtomicUsize>,
    heartbeat: HeartbeatPolicy,
) -> AgentBackend {
    let (r, w) = streams_to(address).unwrap();
    let reconnect: AgentReconnect = Arc::new(move || {
        reconnects.fetch_add(1, Ordering::SeqCst);
        streams_to(address)
    });
    let inner: BackendHandle = Arc::new(LocalBackend::new("/"));
    AgentBackend::from_streams_with_reconnect_and_heartbeat(r, w, inner, reconnect, heartbeat)
        .unwrap()
}

#[test]
fn remote_drive_task_heartbeat_retires_blackholed_live_channel_and_reconnects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let heartbeats = Arc::new(AtomicUsize::new(0));
    let server_heartbeats = heartbeats.clone();
    let server = std::thread::spawn(move || {
        let (mut blackhole, _) = listener.accept().unwrap();
        let mut blackhole_reader = handshake(&mut blackhole);
        let (_, request) = read_frame(&mut blackhole_reader).unwrap().unwrap();
        assert!(matches!(request, Frame::Rename { .. }));
        let (_, heartbeat) = read_frame(&mut blackhole_reader).unwrap().unwrap();
        assert!(matches!(heartbeat, Frame::Hello { .. }));
        server_heartbeats.fetch_add(1, Ordering::SeqCst);

        let (mut replacement, _) = listener.accept().unwrap();
        let mut replacement_reader = handshake(&mut replacement);
        loop {
            let (id, request) = read_frame(&mut replacement_reader).unwrap().unwrap();
            match request {
                Frame::Hello { .. } => write_frame(
                    &mut replacement,
                    id,
                    &Frame::HelloOk {
                        proto: PROTO_VERSION,
                        version: "test".into(),
                    },
                )
                .unwrap(),
                Frame::ListDir(_) => {
                    write_frame(&mut replacement, id, &Frame::Dir(Vec::new())).unwrap();
                    break;
                }
                other => panic!("unexpected replacement request: {other:?}"),
            }
        }
        drop(blackhole_reader);
        drop(blackhole);
    });
    let reconnects = Arc::new(AtomicUsize::new(0));
    let backend = backend(
        address,
        reconnects.clone(),
        HeartbeatPolicy::new(Duration::from_millis(100), Duration::from_millis(500)),
    );

    let started = Instant::now();
    assert!(backend.rename("/source", "/destination").is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(backend.list_dir("/").unwrap().is_empty());
    assert_eq!(heartbeats.load(Ordering::SeqCst), 1);
    assert_eq!(reconnects.load(Ordering::SeqCst), 1);
    drop(backend);
    server.join().unwrap();
}

#[test]
fn remote_drive_task_heartbeat_keeps_responsive_idle_generation_active() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let heartbeats = Arc::new(AtomicUsize::new(0));
    let server_heartbeats = heartbeats.clone();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut reader = handshake(&mut socket);
        loop {
            let (id, request) = read_frame(&mut reader).unwrap().unwrap();
            match request {
                Frame::Hello { .. } => {
                    server_heartbeats.fetch_add(1, Ordering::SeqCst);
                    write_frame(
                        &mut socket,
                        id,
                        &Frame::HelloOk {
                            proto: PROTO_VERSION,
                            version: "test".into(),
                        },
                    )
                    .unwrap();
                }
                Frame::ListDir(_) => {
                    write_frame(&mut socket, id, &Frame::Dir(Vec::new())).unwrap();
                    break;
                }
                other => panic!("unexpected live-channel request: {other:?}"),
            }
        }
    });
    let reconnects = Arc::new(AtomicUsize::new(0));
    let backend = backend(
        address,
        reconnects.clone(),
        HeartbeatPolicy::new(Duration::from_millis(100), Duration::from_millis(500)),
    );

    std::thread::sleep(Duration::from_millis(350));
    assert!(backend.list_dir("/").unwrap().is_empty());
    assert!(heartbeats.load(Ordering::SeqCst) >= 1);
    assert_eq!(reconnects.load(Ordering::SeqCst), 0);
    drop(backend);
    server.join().unwrap();
}

#[test]
fn remote_drive_task_metadata_timeout_drains_old_mutation_without_poisoning_replacement() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release_old_tx, release_old_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut old, _) = listener.accept().unwrap();
        let mut old_reader = handshake(&mut old);
        let mutation_id = loop {
            let (id, request) = read_frame(&mut old_reader).unwrap().unwrap();
            match request {
                Frame::Hello { .. } => write_frame(
                    &mut old,
                    id,
                    &Frame::HelloOk {
                        proto: PROTO_VERSION,
                        version: "test".into(),
                    },
                )
                .unwrap(),
                Frame::Rename { .. } => break id,
                other => panic!("unexpected old-generation request: {other:?}"),
            }
        };
        loop {
            let (id, request) = read_frame(&mut old_reader).unwrap().unwrap();
            match request {
                Frame::Hello { .. } => write_frame(
                    &mut old,
                    id,
                    &Frame::HelloOk {
                        proto: PROTO_VERSION,
                        version: "test".into(),
                    },
                )
                .unwrap(),
                Frame::ListDir(_) => break,
                other => panic!("unexpected old-generation request: {other:?}"),
            }
        }

        let (mut replacement, _) = listener.accept().unwrap();
        let mut replacement_reader = handshake(&mut replacement);
        loop {
            let (id, request) = read_frame(&mut replacement_reader).unwrap().unwrap();
            match request {
                Frame::Hello { .. } => write_frame(
                    &mut replacement,
                    id,
                    &Frame::HelloOk {
                        proto: PROTO_VERSION,
                        version: "test".into(),
                    },
                )
                .unwrap(),
                Frame::ListDir(_) => {
                    write_frame(&mut replacement, id, &Frame::Dir(Vec::new())).unwrap();
                    break;
                }
                other => panic!("unexpected replacement request: {other:?}"),
            }
        }
        release_old_rx.recv().unwrap();
        write_frame(&mut old, mutation_id, &Frame::Ok).unwrap();
        drop(old_reader);
        drop(old);

        loop {
            let (id, request) = read_frame(&mut replacement_reader).unwrap().unwrap();
            match request {
                Frame::Hello { .. } => write_frame(
                    &mut replacement,
                    id,
                    &Frame::HelloOk {
                        proto: PROTO_VERSION,
                        version: "test".into(),
                    },
                )
                .unwrap(),
                Frame::TryExists(_) => {
                    write_frame(&mut replacement, id, &Frame::Exists(true)).unwrap();
                    break;
                }
                other => panic!("unexpected replacement request: {other:?}"),
            }
        }
    });
    let reconnects = Arc::new(AtomicUsize::new(0));
    let reconnect_count = reconnects.clone();
    let reconnect: AgentReconnect = Arc::new(move || {
        reconnect_count.fetch_add(1, Ordering::SeqCst);
        streams_to(address)
    });
    let streams = streams_to(address).unwrap();
    let (connection, _) = AgentConnection::new_with_heartbeat(
        streams,
        Some(reconnect),
        HeartbeatPolicy::new(Duration::from_secs(5), Duration::from_secs(1)),
    )
    .unwrap();

    let old_mux = connection.mutation_mux().unwrap();
    let (mutation_id, mutation_rx) = old_mux.register();
    old_mux
        .send(
            mutation_id,
            Frame::Rename {
                src: "/source".into(),
                dst: "/destination".into(),
            },
        )
        .unwrap();
    let response = connection
        // The old generation intentionally never answers. Leave enough time
        // for a loaded test runner to schedule the replacement handshake and
        // its immediate response; both attempts use this same timeout.
        .safe_call_timeout(Frame::ListDir("/".into()), Duration::from_millis(400))
        .unwrap();
    assert!(matches!(response, Frame::Dir(entries) if entries.is_empty()));
    assert!(old_mux.is_retired());
    assert!(!old_mux.is_closed());
    assert!(matches!(
        mutation_rx.try_recv(),
        Err(crossbeam_channel::TryRecvError::Empty)
    ));
    let replacement_mux = connection.mux().unwrap();
    assert!(!Arc::ptr_eq(&old_mux, &replacement_mux));
    assert_eq!(reconnects.load(Ordering::SeqCst), 1);

    release_old_tx.send(()).unwrap();
    assert_eq!(
        mutation_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        Frame::Ok
    );
    old_mux.unregister(mutation_id);
    assert!(old_mux.is_closed());
    let after_late_old_failure = connection.replace_observed_for_test(&old_mux).unwrap();
    assert!(Arc::ptr_eq(&replacement_mux, &after_late_old_failure));
    std::thread::sleep(Duration::from_millis(150));
    assert!(Arc::ptr_eq(&replacement_mux, &connection.mux().unwrap()));
    assert_eq!(reconnects.load(Ordering::SeqCst), 1);

    let next = connection
        .safe_call_timeout(Frame::TryExists("/".into()), Duration::from_millis(400))
        .unwrap();
    assert!(matches!(next, Frame::Exists(true)));
    assert_eq!(reconnects.load(Ordering::SeqCst), 1);
    drop(connection);
    server.join().unwrap();
}

#[test]
fn remote_drive_task_reconnectless_proxy_survives_one_timed_out_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = handshake(&mut stream);
        loop {
            let (id, request) = read_frame(&mut reader).unwrap().unwrap();
            match request {
                Frame::Hello { .. } => write_frame(
                    &mut stream,
                    id,
                    &Frame::HelloOk {
                        proto: PROTO_VERSION,
                        version: "test".into(),
                    },
                )
                .unwrap(),
                Frame::ListDir(_) => {}
                Frame::TryExists(_) => {
                    write_frame(&mut stream, id, &Frame::Exists(true)).unwrap();
                    break;
                }
                other => panic!("unexpected reconnectless request: {other:?}"),
            }
        }
    });
    let streams = streams_to(address).unwrap();
    let (connection, _) = AgentConnection::new_with_heartbeat(
        streams,
        None,
        HeartbeatPolicy::new(Duration::from_secs(10), Duration::from_secs(1)),
    )
    .unwrap();

    let first = connection
        .safe_call_timeout(Frame::ListDir("/".into()), Duration::from_millis(40))
        .unwrap_err();
    assert_eq!(first.kind(), std::io::ErrorKind::TimedOut);
    let second = connection
        .safe_call_timeout(Frame::TryExists("/".into()), Duration::from_millis(400))
        .unwrap();
    assert!(matches!(second, Frame::Exists(true)));
    drop(connection);
    server.join().unwrap();
}
