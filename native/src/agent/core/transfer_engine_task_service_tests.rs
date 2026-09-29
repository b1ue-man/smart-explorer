//! The background service's backend server, servers without labels and the
//! exec-channel pool, over loopback.
use super::transfer_engine_task_tests::{
    assert_slow_consumer_does_not_block, fwd, served, temp_root, RecordingSink,
};
use super::transport::{AgentReconnect, AgentStreams};
use super::AgentBackend;
use crate::agent_proto::{read_frame, write_frame, Frame, PROTO_VERSION};
use crate::vfs::{
    Backend, BackendHandle, BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink,
    LocalBackend, Scheme, VfsMeta, VfsResult,
};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn serve_service(backend: BackendHandle) -> super::transfer_engine_task_tests::Served {
    served(move |read, socket| {
        let _ = crate::daemon::serve_sync_link_fixture(read, socket, backend);
    })
}

/// A peer with batches: two files per batch, written through the local
/// filesystem.
struct BatchingLocal {
    inner: LocalBackend,
    puts: AtomicUsize,
}

impl Backend for BatchingLocal {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
    fn batch_limits(&self, _dir: &str) -> Option<BatchLimits> {
        Some(BatchLimits {
            max_files: 2,
            max_bytes: 1 << 20,
        })
    }
    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        let mut outcomes = Vec::new();
        for entry in entries {
            let mut bytes = vec![0u8; entry.size as usize];
            data.read_exact(&mut bytes)?;
            let mut writer = self.inner.open_write_new(&entry.path)?;
            writer.write_all(&bytes)?;
            writer.flush()?;
            outcomes.push(BatchPutOutcome::Published(entry.path.clone()));
        }
        Ok(outcomes)
    }
    fn get_batch(&self, items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
        for (index, item) in items.iter().enumerate() {
            let mut bytes = Vec::new();
            let read = self
                .inner
                .open_read(&item.path)
                .and_then(|mut reader| reader.read_to_end(&mut bytes));
            match read {
                Ok(_) => {
                    sink.begin(index, bytes.len() as u64)?;
                    sink.data(index, &bytes)?;
                    sink.end(index, Ok(()))?;
                }
                Err(error) => sink.failed(index, error)?,
            }
        }
        Ok(())
    }
}

#[test]
fn transfer_engine_task_service_slow_consumer_does_not_block_a_parallel_listing() {
    let root = temp_root("service_slow");
    let served = serve_service(Arc::new(LocalBackend::new("/")));
    let features = served.backend.features();
    assert!(features.credit && features.service);
    // The local peer has no batches: the service does not announce them.
    assert!(served.backend.batch_limits(&fwd(&root)).is_none());
    // The service's slots minus its browsing reserve of four.
    let peer_ceiling = LocalBackend::new("/").transfer_ceiling("/");
    let expected = crate::agent_proto::service_slots(peer_ceiling) - 4;
    assert_eq!(served.backend.transfer_ceiling("/"), Some(expected));
    assert_slow_consumer_does_not_block(&served.backend, &root);
    served.finish();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn transfer_engine_task_service_forwards_batches_split_by_peer_limits() {
    let root = temp_root("service_batch");
    let peer = Arc::new(BatchingLocal {
        inner: LocalBackend::new("/"),
        puts: AtomicUsize::new(0),
    });
    let served = serve_service(peer.clone());
    assert!(served.backend.batch_limits(&fwd(&root)).is_some());
    let entries: Vec<BatchPut> = [("x", 1), ("y", 2), ("z", 3)]
        .iter()
        .map(|(name, size)| BatchPut {
            path: fwd(&root.join(name)),
            size: *size,
        })
        .collect();
    let mut data: &[u8] = b"xyyzzz";
    let outcomes = served.backend.put_batch(&entries, &mut data).unwrap();
    for (outcome, entry) in outcomes.iter().zip(&entries) {
        assert!(matches!(outcome, BatchPutOutcome::Published(path) if *path == entry.path));
    }
    assert_eq!(peer.puts.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(root.join("z")).unwrap(), b"zzz");

    let items: Vec<BatchGet> = ["x", "y", "z", "missing"]
        .iter()
        .map(|name| BatchGet {
            path: fwd(&root.join(name)),
            id: None,
            size: 3,
        })
        .collect();
    let mut sink = RecordingSink::default();
    served.backend.get_batch(&items, &mut sink).unwrap();
    assert_eq!(sink.begun, vec![(0, 1), (1, 2), (2, 3)]);
    assert_eq!(sink.data[&1], b"yy");
    assert_eq!(sink.failed, vec![3]);
    served.finish();
    let _ = std::fs::remove_dir_all(root);
}

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
    assert!(backend
        .server_copy_to_stage(
            "/a",
            "/a.se-copy-1",
            1,
            &std::sync::atomic::AtomicBool::new(false)
        )
        .unwrap()
        .is_none());
    assert_eq!(
        backend
            .discard_copy_stage("/a.se-copy-1")
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
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
