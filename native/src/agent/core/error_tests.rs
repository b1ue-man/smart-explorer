use super::transport::{AgentReconnect, AgentStreams};
use super::AgentBackend;
use crate::agent_proto::{read_frame, write_frame, Frame, SearchSpec, PROTO_VERSION};
use crate::vfs::{Backend, BackendHandle, HashHit, Scheme, SearchHit, VfsMeta, VfsResult};
use std::io::{self, Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

#[derive(Default)]
struct TrackingInner {
    lists: AtomicUsize,
    renames: AtomicUsize,
}

impl Backend for TrackingInner {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.lists.fetch_add(1, Ordering::Relaxed);
        Ok(Vec::new())
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        Err(io::Error::new(io::ErrorKind::NotFound, path))
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(Cursor::new(Vec::<u8>::new())))
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(Vec::<u8>::new()))
    }

    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        self.renames.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(())
    }
}

fn scripted_agent(
    inner: BackendHandle,
    script: impl FnOnce(u64, Frame, &mut TcpStream) + Send + 'static,
) -> (AgentBackend, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut reader = socket.try_clone().unwrap();
        let (hello_id, hello) = read_frame(&mut reader).unwrap().unwrap();
        assert!(matches!(hello, Frame::Hello { .. }));
        write_frame(
            &mut socket,
            hello_id,
            &Frame::HelloOk {
                proto: PROTO_VERSION,
                version: "test".into(),
            },
        )
        .unwrap();
        let (request_id, request) = read_frame(&mut reader).unwrap().unwrap();
        script(request_id, request, &mut socket);
    });
    let client = TcpStream::connect(address).unwrap();
    let backend = AgentBackend::from_streams(
        Box::new(client.try_clone().unwrap()),
        Box::new(client),
        inner,
    )
    .unwrap();
    (backend, server)
}

fn streams_to(address: SocketAddr) -> io::Result<AgentStreams> {
    let stream = TcpStream::connect(address)?;
    Ok((Box::new(stream.try_clone()?), Box::new(stream)))
}

fn handshake(socket: &mut TcpStream) -> TcpStream {
    let mut reader = socket.try_clone().unwrap();
    let (hello_id, hello) = read_frame(&mut reader).unwrap().unwrap();
    assert!(matches!(hello, Frame::Hello { .. }));
    write_frame(
        socket,
        hello_id,
        &Frame::HelloOk {
            proto: PROTO_VERSION,
            version: "test".into(),
        },
    )
    .unwrap();
    reader
}

fn reconnectable_backend(
    address: SocketAddr,
    inner: BackendHandle,
    reconnects: Arc<AtomicUsize>,
) -> AgentBackend {
    let (r, w) = streams_to(address).unwrap();
    let reconnect: AgentReconnect = Arc::new(move || {
        reconnects.fetch_add(1, Ordering::SeqCst);
        streams_to(address)
    });
    AgentBackend::from_streams_with_reconnect(r, w, inner, reconnect).unwrap()
}

#[test]
fn safe_read_reconnects_once_after_closed_generation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for generation in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = handshake(&mut socket);
            let (request_id, request) = read_frame(&mut reader).unwrap().unwrap();
            assert!(matches!(request, Frame::ListDir(_)));
            if generation == 1 {
                write_frame(&mut socket, request_id, &Frame::Dir(Vec::new())).unwrap();
                release_rx.recv().unwrap();
            }
        }
    });
    let reconnects = Arc::new(AtomicUsize::new(0));
    let backend = reconnectable_backend(
        address,
        Arc::new(TrackingInner::default()),
        reconnects.clone(),
    );

    assert!(backend.list_dir("/").unwrap().is_empty());
    assert_eq!(reconnects.load(Ordering::SeqCst), 1);
    drop(backend);
    release_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn stream_read_reconnects_only_before_bytes_are_returned() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for generation in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = handshake(&mut socket);
            let (request_id, request) = read_frame(&mut reader).unwrap().unwrap();
            assert!(matches!(request, Frame::Read { .. }));
            if generation == 1 {
                write_frame(&mut socket, request_id, &Frame::Data(b"complete".to_vec())).unwrap();
                write_frame(&mut socket, request_id, &Frame::End).unwrap();
                release_rx.recv().unwrap();
            }
        }
    });
    let reconnects = Arc::new(AtomicUsize::new(0));
    let backend = reconnectable_backend(
        address,
        Arc::new(TrackingInner::default()),
        reconnects.clone(),
    );

    let mut reader = backend.open_read("/file").unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"complete");
    assert_eq!(reconnects.load(Ordering::SeqCst), 1);
    drop(reader);
    drop(backend);
    release_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn stream_read_never_restarts_after_returning_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut reader = handshake(&mut socket);
        let (request_id, request) = read_frame(&mut reader).unwrap().unwrap();
        assert!(matches!(request, Frame::Read { .. }));
        write_frame(&mut socket, request_id, &Frame::Data(b"partial".to_vec())).unwrap();
    });
    let backend = reconnectable_backend(
        address,
        Arc::new(TrackingInner::default()),
        Arc::new(AtomicUsize::new(0)),
    );

    let mut reader = backend.open_read("/file").unwrap();
    let mut first = [0u8; 7];
    assert_eq!(reader.read(&mut first).unwrap(), 7);
    assert_eq!(&first, b"partial");
    assert!(reader.read(&mut [0u8; 1]).is_err());
    drop(reader);
    drop(backend);
    server.join().unwrap();
}

#[test]
fn safe_read_retries_no_more_than_one_generation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = handshake(&mut socket);
            let (_, request) = read_frame(&mut reader).unwrap().unwrap();
            assert!(matches!(request, Frame::ListDir(_)));
            server_requests.fetch_add(1, Ordering::SeqCst);
        }
    });
    let reconnects = Arc::new(AtomicUsize::new(0));
    let backend = reconnectable_backend(
        address,
        Arc::new(TrackingInner::default()),
        reconnects.clone(),
    );

    assert!(backend.list_dir("/").is_err());
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    drop(backend);
    server.join().unwrap();
}

#[test]
fn committed_mutation_is_not_replayed_across_keepalive_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let commits = Arc::new(AtomicUsize::new(0));
    let server_commits = commits.clone();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        let mut first_reader = handshake(&mut first);
        let (_, request) = read_frame(&mut first_reader).unwrap().unwrap();
        assert!(matches!(request, Frame::Rename { .. }));
        server_commits.fetch_add(1, Ordering::SeqCst);
        drop(first_reader);
        drop(first);

        let (mut replacement, _) = listener.accept().unwrap();
        let mut replacement_reader = handshake(&mut replacement);
        let (request_id, request) = read_frame(&mut replacement_reader).unwrap().unwrap();
        assert!(matches!(request, Frame::ListDir(_)));
        write_frame(&mut replacement, request_id, &Frame::Dir(Vec::new())).unwrap();
        release_rx.recv().unwrap();
    });
    let inner = Arc::new(TrackingInner::default());
    let reconnects = Arc::new(AtomicUsize::new(0));
    let backend = reconnectable_backend(address, inner.clone(), reconnects.clone());

    assert!(backend.rename("/source", "/destination").is_err());
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert_eq!(inner.renames.load(Ordering::SeqCst), 0);

    assert!(backend.list_dir("/").unwrap().is_empty());
    assert_eq!(reconnects.load(Ordering::SeqCst), 1);
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    drop(backend);
    release_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn committed_mutation_disconnect_never_invokes_inner_backend() {
    let inner = Arc::new(TrackingInner::default());
    let committed = Arc::new(AtomicBool::new(false));
    let committed_server = committed.clone();
    let (backend, server) = scripted_agent(inner.clone(), move |_, request, _| {
        assert!(matches!(request, Frame::Rename { .. }));
        committed_server.store(true, Ordering::Release);
        // Simulate a server that committed but lost the acknowledgement.
    });

    let error = backend.rename("/source", "/destination").unwrap_err();
    assert!(committed.load(Ordering::Acquire));
    assert!(matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe
    ));
    assert_eq!(inner.renames.load(Ordering::Relaxed), 0);
    drop(backend);
    server.join().unwrap();
}

#[test]
fn lost_write_ack_stays_failed_on_every_flush_without_replay() {
    let (backend, server) = scripted_agent(
        Arc::new(TrackingInner::default()),
        |request_id, request, socket| {
            assert!(matches!(request, Frame::Write(_)));
            write_frame(socket, request_id, &Frame::Progress { done: 0, total: 0 }).unwrap();
            let mut reader = socket.try_clone().unwrap();
            let (end_id, end) = read_frame(&mut reader).unwrap().unwrap();
            assert_eq!(end_id, request_id);
            assert!(matches!(end, Frame::End));
            // The remote commit happened, but its acknowledgement is lost.
        },
    );
    let mut writer = backend.open_write("/destination").unwrap();

    let first = writer.flush().unwrap_err();
    let second = writer.flush().unwrap_err();
    assert_eq!(second.kind(), first.kind());
    assert_eq!(second.to_string(), first.to_string());
    drop(writer);
    drop(backend);
    server.join().unwrap();
}

#[test]
fn search_hit_followed_by_error_propagates_without_listing_fallback() {
    let inner = Arc::new(TrackingInner::default());
    let (backend, server) = scripted_agent(inner.clone(), |id, request, socket| {
        assert!(matches!(request, Frame::Search { .. }));
        write_frame(
            socket,
            id,
            &Frame::Match {
                rel: "partial.txt".into(),
                is_dir: false,
                size: 7,
                mtime_ms: 1,
            },
        )
        .unwrap();
        write_frame(socket, id, &Frame::Err("search failed late".into())).unwrap();
    });
    let (tx, rx) = crossbeam_channel::bounded::<SearchHit>(4);
    let spec = SearchSpec {
        query: String::new(),
        glob: false,
        min_size: 0,
        max_size: 0,
        max_results: 0,
        want_dirs: false,
    };

    let error = search_or_fallback(&backend, &spec, tx).unwrap_err();
    assert!(error.to_string().contains("search failed late"));
    assert_eq!(rx.iter().count(), 1);
    assert_eq!(inner.lists.load(Ordering::Relaxed), 0);
    drop(backend);
    server.join().unwrap();
}

#[test]
fn hash_hit_followed_by_error_propagates_without_listing_fallback() {
    let inner = Arc::new(TrackingInner::default());
    let (backend, server) = scripted_agent(inner.clone(), |id, request, socket| {
        assert!(matches!(request, Frame::WalkHashed { .. }));
        write_frame(
            socket,
            id,
            &Frame::HashEntry {
                rel: "partial.txt".into(),
                is_dir: false,
                size: 7,
                mtime_ms: 1,
                md5: Some("900150983cd24fb0d6963f7d28e17f72".into()),
            },
        )
        .unwrap();
        write_frame(socket, id, &Frame::Err("hash walk failed late".into())).unwrap();
    });
    let (tx, rx) = crossbeam_channel::bounded::<HashHit>(4);

    let error = hash_walk_or_fallback(&backend, tx).unwrap_err();
    assert!(error.to_string().contains("hash walk failed late"));
    assert_eq!(rx.iter().count(), 1);
    assert_eq!(inner.lists.load(Ordering::Relaxed), 0);
    drop(backend);
    server.join().unwrap();
}

fn search_or_fallback(
    backend: &dyn Backend,
    spec: &SearchSpec,
    tx: crossbeam_channel::Sender<SearchHit>,
) -> io::Result<()> {
    if backend.search("/", spec, tx, &AtomicBool::new(false))? {
        Ok(())
    } else {
        backend.list_dir("/").map(|_| ())
    }
}

fn hash_walk_or_fallback(
    backend: &dyn Backend,
    tx: crossbeam_channel::Sender<HashHit>,
) -> io::Result<()> {
    if backend.walk_hashed("/", true, tx, &AtomicBool::new(false))? {
        Ok(())
    } else {
        backend.list_dir("/").map(|_| ())
    }
}
