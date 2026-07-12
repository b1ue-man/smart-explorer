use super::transport::{AgentReconnect, AgentStreams, HeartbeatPolicy};
use super::AgentBackend;
use crate::agent_proto::{read_frame, write_frame, Frame, PROTO_VERSION};
use crate::vfs::{Backend, BackendHandle, LocalBackend};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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
fn heartbeat_retires_blackholed_live_channel_and_reconnects() {
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
fn heartbeat_keeps_responsive_idle_generation_active() {
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
