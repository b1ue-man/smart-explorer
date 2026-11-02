//! Servers without labels and the exec-channel pool, over loopback. The service
//! tests that serve through the daemon's sync-link fixture moved to
//! daemon/os/shared/transfer_engine_task_service_tests.rs (plan D13).
use super::transfer_engine_task_tests::{fwd, temp_root};
use super::transport::{AgentReconnect, AgentStreams};
use super::AgentBackend;
use crate::agent_proto::{read_frame, write_frame, Frame, PROTO_VERSION};
use crate::vfs::{Backend, LocalBackend};
use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn answer_hello(socket: &mut TcpStream, reader: &mut TcpStream, version: &str) {
    let (id, hello) = read_frame(reader).unwrap().unwrap();
    assert!(matches!(hello, Frame::Hello { .. }));
    write_frame(
        socket,
        id,
        &Frame::HelloOk {
            proto: PROTO_VERSION,
            version: version.into(),
        },
    )
    .unwrap();
}

#[test]
fn transfer_engine_task_server_without_labels_keeps_the_single_file_path() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut reader = socket.try_clone().unwrap();
        answer_hello(&mut socket, &mut reader, "0.1.0+sync-links-v1");
        // No credit switch, no new request: the former frames only.
        let (id, request) = read_frame(&mut reader).unwrap().unwrap();
        assert!(matches!(request, Frame::Mkdir(_)), "{request:?}");
        write_frame(&mut socket, id, &Frame::Ok).unwrap();
        let (id, request) = read_frame(&mut reader).unwrap().unwrap();
        assert!(matches!(request, Frame::TryExists(_)), "{request:?}");
        write_frame(&mut socket, id, &Frame::Exists(true)).unwrap();
        let (id, request) = read_frame(&mut reader).unwrap().unwrap();
        assert!(
            matches!(request, Frame::Read { offset: 5, .. }),
            "{request:?}"
        );
        write_frame(&mut socket, id, &Frame::Data(b"tail".to_vec())).unwrap();
        write_frame(&mut socket, id, &Frame::End).unwrap();
        while let Ok(Some((_, frame))) = read_frame(&mut reader) {
            assert!(matches!(frame, Frame::Hello { .. }), "{frame:?}");
        }
    });
    let client = TcpStream::connect(address).unwrap();
    let shutdown = client.try_clone().unwrap();
    let backend = AgentBackend::from_streams(
        Box::new(client.try_clone().unwrap()),
        Box::new(client),
        Arc::new(LocalBackend::new("/")),
    )
    .unwrap();
    assert!(!backend.features().credit);
    assert!(backend.batch_limits("/").is_none());
    assert_eq!(
        backend.put_batch(&[], &mut io::empty()).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    // Server copies and their cleanup go to the connection underneath (the
    // local filesystem here, SFTP `copy-data` over SSH), never as agent frames.
    let root = temp_root("old_agent_stage");
    std::fs::write(root.join("a"), b"abc").unwrap();
    let stage = fwd(&root.join("a.se-copy-1"));
    let copied = backend
        .server_copy_to_stage(
            &fwd(&root.join("a")),
            &stage,
            3,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(copied, Some(3));
    backend.discard_copy_stage(&stage).unwrap();
    assert!(!root.join("a.se-copy-1").exists());
    let _ = std::fs::remove_dir_all(&root);
    backend.create_dir("/new").unwrap();
    assert_eq!(
        backend.create_dir_new("/existing").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let mut tail = String::new();
    backend
        .open_read_at("/file", None, 5)
        .unwrap()
        .expect("offset reads")
        .read_to_string(&mut tail)
        .unwrap();
    assert_eq!(tail, "tail");
    assert_eq!(backend.transfer_ceiling("/"), Some(6));
    drop(backend);
    let _ = shutdown.shutdown(std::net::Shutdown::Both);
    server.join().unwrap();
}

/// Streams to `address`; every socket is kept for the final shutdown.
fn streams_to(address: SocketAddr, sockets: &Mutex<Vec<TcpStream>>) -> io::Result<AgentStreams> {
    let stream = TcpStream::connect(address)?;
    sockets
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(stream.try_clone()?);
    Ok((Box::new(stream.try_clone()?), Box::new(stream)))
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn transfer_engine_task_channel_pool_grows_and_learns_the_refused_limit() {
    let root = temp_root("pool");
    let big = vec![7u8; 16 * 1024 * 1024];
    std::fs::write(root.join("big.bin"), &big).unwrap();
    std::fs::write(root.join("small.txt"), b"s").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut sessions = Vec::new();
        for socket in listener.incoming().take(2) {
            let socket = socket.unwrap();
            let read = socket.try_clone().unwrap();
            sessions.push(std::thread::spawn(move || {
                let _ = crate::agent_proto::serve(read, socket);
            }));
        }
        for session in sessions {
            let _ = session.join();
        }
    });
    let opened = Arc::new(AtomicUsize::new(0));
    let sockets = Arc::new(Mutex::new(Vec::new()));
    let counter = opened.clone();
    let opener_sockets = sockets.clone();
    // The server takes one more session, then refuses like MaxSessions.
    let opener: AgentReconnect = Arc::new(move || {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            streams_to(address, &opener_sockets)
        } else {
            Err(io::Error::other("administratively prohibited: open failed"))
        }
    });
    let (r, w) = streams_to(address, &sockets).unwrap();
    let backend =
        AgentBackend::pooled_for_test(r, w, Arc::new(LocalBackend::new("/")), opener).unwrap();
    assert_eq!(backend.transfer_ceiling("/"), None);
    let big_path = fwd(&root.join("big.bin"));

    let mut first = backend.open_read(&big_path).unwrap();
    let mut byte = [0u8; 1];
    first.read_exact(&mut byte).unwrap();
    assert_eq!(backend.list_dir(&fwd(&root)).unwrap().len(), 2);
    wait_until("a second channel", || backend.channel_count() == 2);

    let mut second = backend.open_read(&big_path).unwrap();
    second.read_exact(&mut byte).unwrap();
    assert_eq!(backend.stat(&fwd(&root.join("small.txt"))).unwrap().size, 1);
    wait_until("the learned limit", || {
        backend.transfer_ceiling("/").is_some()
    });
    assert_eq!(opened.load(Ordering::SeqCst), 2);
    assert_eq!(backend.channel_count(), 1);
    assert_eq!(backend.transfer_ceiling("/"), Some(62));

    // Both streams finish, also the one on the channel the pool gave up.
    for mut reader in [first, second] {
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).unwrap();
        assert_eq!(rest.len() + 1, big.len());
    }
    drop(backend);
    for socket in sockets.lock().unwrap().iter() {
        let _ = socket.shutdown(std::net::Shutdown::Both);
    }
    let _ = server.join();
    let _ = std::fs::remove_dir_all(root);
}
