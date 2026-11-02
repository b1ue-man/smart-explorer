use crate::analytics::{Progress, ScanOutcome, ScanStatus, SizeNode};
use crate::daemon::{
    backend_server, ipc_analysis, ipc_client, ipc_protocol::IpcRequest, ipc_storage,
};
use crate::share::{CopyPastePeerFixture, PeerOpenTarget};
use crate::vfs::{Backend, BackendHandle, CachingBackend, Scheme, VfsMeta};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

struct AnalysisOnly {
    peer: BackendHandle,
    scans: Arc<AtomicU64>,
    file_calls: Arc<AtomicU64>,
}
impl Backend for AnalysisOnly {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn scan_storage(&self, root: &str, p: &Progress) -> io::Result<Option<ScanOutcome>> {
        self.scans.fetch_add(1, Ordering::Relaxed);
        self.peer.scan_storage(root, p)
    }
    fn list_dir(&self, _: &str) -> io::Result<Vec<VfsMeta>> {
        self.file_calls.fetch_add(1, Ordering::Relaxed);
        Err(unused())
    }
    fn stat(&self, _: &str) -> io::Result<VfsMeta> {
        self.file_calls.fetch_add(1, Ordering::Relaxed);
        Err(unused())
    }
    fn open_read(&self, _: &str) -> io::Result<Box<dyn Read + Send>> {
        Err(unused())
    }
    fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> {
        Err(unused())
    }
    fn rename(&self, _: &str, _: &str) -> io::Result<()> {
        Err(unused())
    }
    fn remove_file(&self, _: &str) -> io::Result<()> {
        Err(unused())
    }
    fn remove_dir(&self, _: &str) -> io::Result<()> {
        Err(unused())
    }
    fn mkdir_all(&self, _: &str) -> io::Result<()> {
        Err(unused())
    }
}
fn unused() -> io::Error {
    io::Error::other("analysis must not issue individual metadata/file requests")
}

struct Bridge {
    backend: BackendHandle,
    stop: TcpStream,
    address: std::net::SocketAddr,
    workers: Vec<thread::JoinHandle<io::Result<()>>>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.stop.shutdown(Shutdown::Both);
        // Wake the listener too if the caller failed before opening analysis.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
        ipc_storage::clear_ipc_addr();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn bridge(peer: BackendHandle) -> io::Result<Bridge> {
    assert_eq!(
        std::env::var("SMART_EXPLORER_WINDOWS_REMOTE_TASK").as_deref(),
        Ok("1")
    );
    assert!(
        ipc_storage::read_ipc_addr().is_none(),
        "suite requires its isolated APPDATA profile"
    );
    let token = ipc_storage::load_or_create_token()?;
    let target = PeerOpenTarget::Direct {
        contact_id: "task-direct-identity".into(),
    };
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    ipc_storage::write_ipc_addr(address)?;
    let analysis_peer = peer.clone();
    let analysis = thread::spawn(move || {
        let (stream, _) = listener.accept()?;
        let mut line = String::new();
        std::io::BufReader::new(stream.try_clone()?).read_line(&mut line)?;
        let request: IpcRequest = serde_json::from_str(&line).map_err(io::Error::other)?;
        super::require_token(&token, request.daemon_token().unwrap_or(""))?;
        let IpcRequest::AnalyzeShare {
            target,
            root,
            node_budget,
            ..
        } = request
        else {
            return Err(unused());
        };
        assert!(
            matches!(target, PeerOpenTarget::Direct { contact_id } if contact_id == "task-direct-identity")
        );
        ipc_analysis::serve(stream, root, node_budget, || Ok(analysis_peer))
    });
    let ordinary = TcpListener::bind("127.0.0.1:0")?;
    let client = TcpStream::connect(ordinary.local_addr()?)?;
    let stop = client.try_clone()?;
    let (server, _) = ordinary.accept()?;
    let read = server.try_clone()?;
    let server = thread::spawn(move || backend_server::serve_backend(read, server, peer));
    let identity = ipc_client::share_backend_identity("task share".into(), target);
    let agent = crate::agent::AgentBackend::from_streams(
        Box::new(client.try_clone()?),
        Box::new(client),
        identity,
    )?;
    Ok(Bridge {
        backend: Arc::new(CachingBackend::new(Arc::new(agent))),
        stop,
        address,
        workers: vec![analysis, server],
    })
}

fn flatten(tree: &SizeNode) -> BTreeMap<String, (u64, bool)> {
    let mut result = BTreeMap::new();
    let mut stack = vec![(String::new(), tree)];
    while let Some((path, node)) = stack.pop() {
        result.insert(path.clone(), (node.size, node.is_dir));
        for child in &node.children {
            stack.push((format!("{path}/{}", child.name), child));
        }
    }
    result
}

#[test]
fn windows_remote_task_analysis_matches_local_through_gui_worker_and_cache() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::new()?;
    let wide = fixture.root_a.join("wide");
    std::fs::create_dir(&wide)?;
    for index in 0..5000 {
        std::fs::write(
            wide.join(format!("file-{index:05}")),
            vec![0; index % 11 + 1],
        )?;
    }
    for index in 0..16 {
        let path = fixture.root_a.join(format!("branch-{index}/nested"));
        std::fs::create_dir_all(&path)?;
        for file in 0..16 {
            std::fs::write(path.join(format!("{file}.bin")), b"branch")?;
        }
    }
    let mut deep = fixture.root_a.join("deep");
    for _ in 0..32 {
        deep = deep.join("d");
        std::fs::create_dir_all(&deep)?;
        std::fs::write(deep.join("file"), b"deep")?;
    }
    std::fs::create_dir(fixture.root_a.join("node_modules"))?;
    std::fs::write(fixture.root_a.join("node_modules/kept"), b"kept")?;

    let local_progress = Progress::default();
    let start = Instant::now();
    let local = crate::analytics::scan(&fixture.root_a, &local_progress);
    let local_time = start.elapsed();
    assert_eq!(local.status, ScanStatus::Complete);
    assert!(local.aggregated_files > 0);
    let expected = local_progress.snapshot();
    let scans = Arc::new(AtomicU64::new(0));
    let file_calls = Arc::new(AtomicU64::new(0));
    let guarded = Arc::new(AnalysisOnly {
        peer: fixture.backend.clone(),
        scans: scans.clone(),
        file_calls: file_calls.clone(),
    });
    let bridge = bridge(guarded)?;
    let progress = Progress::default();
    let start = Instant::now();
    let remote = crate::analytics::scan_remote(&*bridge.backend, "/A", &progress);
    let elapsed = start.elapsed();
    assert_eq!(remote.status, local.status, "{:?}", remote.issues);
    assert_eq!(remote.aggregated_files, local.aggregated_files);
    assert_eq!(remote.permission_denied, local.permission_denied);
    assert_eq!(
        flatten(remote.tree.as_ref().unwrap()),
        flatten(local.tree.as_ref().unwrap())
    );
    let actual = progress.snapshot();
    assert_eq!(
        (actual.files, actual.dirs, actual.bytes),
        (expected.files, expected.dirs, expected.bytes)
    );
    assert_eq!(actual.files, 5000 + 256 + 32 + 1);
    assert_eq!(scans.load(Ordering::Relaxed), 1);
    assert_eq!(
        file_calls.load(Ordering::Relaxed),
        0,
        "worker reintroduced per-file network traversal"
    );
    assert!(
        progress.report_count() <= elapsed.as_millis() as u64 / 250 + 8,
        "progress traffic scaled with file count"
    );
    let host_ms = actual
        .host_scan_ms
        .expect("measured host duration must survive IPC");
    eprintln!("ANALYSIS_TIMING files={} local_ms={} host_ms={} gui_end_to_end_ms={} metadata_calls=0 progress_reports={}",
        actual.files, local_time.as_millis(), host_ms, elapsed.as_millis(), progress.report_count());
    assert!(
        u128::from(host_ms) <= local_time.as_millis() * 2 + 500,
        "host scan regressed against identical local worker"
    );
    assert!(
        elapsed.as_millis() <= local_time.as_millis() * 3 + 2500,
        "loopback GUI/worker transport stalled"
    );
    Ok(())
}

#[test]
fn windows_remote_task_analysis_ipc_cancellation_reaches_active_worker() -> io::Result<()> {
    struct Waiting(Arc<std::sync::atomic::AtomicBool>);
    impl Backend for Waiting {
        fn scheme(&self) -> Scheme {
            Scheme::Peer
        }
        fn root_display(&self) -> String {
            "/".into()
        }
        fn scan_storage(&self, _: &str, p: &Progress) -> io::Result<Option<ScanOutcome>> {
            self.0.store(true, Ordering::Relaxed);
            while p.check_cancel().is_ok() {
                thread::sleep(Duration::from_millis(5));
            }
            self.0.store(false, Ordering::Relaxed);
            Ok(Some(ScanOutcome::canceled()))
        }
        fn list_dir(&self, _: &str) -> io::Result<Vec<VfsMeta>> {
            Err(unused())
        }
        fn stat(&self, _: &str) -> io::Result<VfsMeta> {
            Err(unused())
        }
        fn open_read(&self, _: &str) -> io::Result<Box<dyn Read + Send>> {
            Err(unused())
        }
        fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> {
            Err(unused())
        }
        fn rename(&self, _: &str, _: &str) -> io::Result<()> {
            Err(unused())
        }
        fn remove_file(&self, _: &str) -> io::Result<()> {
            Err(unused())
        }
        fn remove_dir(&self, _: &str) -> io::Result<()> {
            Err(unused())
        }
        fn mkdir_all(&self, _: &str) -> io::Result<()> {
            Err(unused())
        }
    }
    let active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let bridge = bridge(Arc::new(Waiting(active.clone())))?;
    let backend = bridge.backend.clone();
    let progress = Progress::default();
    let worker_progress = progress.clone();
    let worker =
        thread::spawn(move || crate::analytics::scan_remote(&*backend, "/A", &worker_progress));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !active.load(Ordering::Relaxed) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(active.load(Ordering::Relaxed));
    let start = Instant::now();
    progress.cancel.store(true, Ordering::Relaxed);
    let outcome = worker.join().unwrap();
    assert_eq!(outcome.status, ScanStatus::Canceled);
    while active.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(2) {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !active.load(Ordering::Relaxed),
        "actual worker survived the canceled IPC request"
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    let request = IpcRequest::AnalyzeShare {
        token: "correct".into(),
        target: PeerOpenTarget::Direct {
            contact_id: "identity".into(),
        },
        root: "/literal root".into(),
        node_budget: Some(progress.node_budget()),
    };
    assert_eq!(request.daemon_token(), Some("correct"));
    assert_eq!(
        super::require_token("wrong", request.daemon_token().unwrap())
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    Ok(())
}
