use super::AgentBackend;
use crate::agent_proto::{read_frame, write_frame, Frame, SearchSpec, PROTO_VERSION};
use crate::vfs::{Backend, BackendHandle, HashHit, Scheme, SearchHit, VfsMeta, VfsResult};
use std::io::{self, Cursor, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

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
