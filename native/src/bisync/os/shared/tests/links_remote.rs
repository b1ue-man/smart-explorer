use super::super::*;
use super::fwd;
use crate::agent::AgentBackend;
use crate::vfs::{Backend, BackendHandle, LocalBackend};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{atomic::AtomicBool, Arc};

struct Endpoint {
    backend: AgentBackend,
    socket: TcpStream,
    server: Option<std::thread::JoinHandle<()>>,
}

impl Endpoint {
    fn new(root: &str, daemon: bool) -> Self {
        Self::connect(root, move |reader, writer, served| {
            if daemon {
                let _ = crate::daemon::serve_sync_link_fixture(reader, writer, served);
            } else {
                let _ = crate::agent_proto::serve(reader, writer);
            }
        })
    }

    fn connect(root: &str, handler: impl FnOnce(TcpStream, TcpStream, BackendHandle) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let inner: BackendHandle = Arc::new(LocalBackend::new(root));
        let served = inner.clone();
        let server = std::thread::spawn(move || {
            let (writer, _) = listener.accept().unwrap();
            let reader = writer.try_clone().unwrap();
            handler(reader, writer, served);
        });
        let socket = TcpStream::connect(address).unwrap();
        let backend = AgentBackend::from_streams(
            Box::new(socket.try_clone().unwrap()), Box::new(socket.try_clone().unwrap()), inner,
        ).unwrap();
        Self { backend, socket, server: Some(server) }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
        if let Some(server) = self.server.take() { server.join().unwrap(); }
    }
}

#[test]
fn sync_links_task_agent_and_daemon_streams_fall_back_without_losing_protection() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private.txt"), b"never read or replace").unwrap();
    link_fixture::directory(outside.path(), &a.path().join("node_modules"));
    link_fixture::directory(outside.path(), &b.path().join("target_link"));
    std::fs::create_dir(b.path().join("node_modules")).unwrap();
    std::fs::write(b.path().join("node_modules/keep.txt"), b"protected").unwrap();
    std::fs::create_dir(a.path().join("target_link")).unwrap();
    std::fs::write(a.path().join("target_link/source.txt"), b"stay here").unwrap();
    std::fs::write(a.path().join("from-a.txt"), b"agent source").unwrap();
    std::fs::write(b.path().join("from-b.txt"), b"daemon source").unwrap();
    let (ra, rb) = (fwd(a.path()), fwd(b.path()));
    let ea = Endpoint::new(&ra, false);
    let eb = Endpoint::new(&rb, true);
    let cancel = AtomicBool::new(false);
    // The exact marker must survive the real framed transport on both servers.
    for (endpoint, root) in [(&ea, &ra), (&eb, &rb)] {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let error = endpoint.backend.walk_hashed(root, true, tx, &cancel).unwrap_err();
        assert_eq!(error.to_string(), crate::agent_proto::HASH_WALK_LINK_BOUNDARY);
    }
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let opts = BisyncOptions { compare: CompareMode::Checksum, ..Default::default() };
    let result = super::super::run(&ea.backend, &ra, &eb.backend, &rb, opts, &cancel, &filter);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.conflicts.is_empty());
    assert_eq!(result.omissions.reported_paths().collect::<Vec<_>>(), ["node_modules", "target_link"]);
    assert_eq!(std::fs::read(b.path().join("from-a.txt")).unwrap(), b"agent source");
    assert_eq!(std::fs::read(a.path().join("from-b.txt")).unwrap(), b"daemon source");
    assert_eq!(std::fs::read(b.path().join("node_modules/keep.txt")).unwrap(), b"protected");
    assert!(!b.path().join("node_modules/private.txt").exists());
    assert!(!outside.path().join("source.txt").exists());
    assert!(result.baseline.keys().all(|path| !path.starts_with("node_modules/")
        && !path.starts_with("target_link/")));
    let pair = pair_id_for(&ea.backend, &ra, &eb.backend, &rb);
    std::fs::remove_file(baseline_path(&pair)).unwrap();
    link_fixture::remove_directory(&a.path().join("node_modules"));
    link_fixture::remove_directory(&b.path().join("target_link"));
}

#[test]
fn sync_links_task_regular_agent_tree_keeps_fast_hash_path_and_filters() {
    let directory = tempfile::tempdir().unwrap();
    for folder in ["node_modules", ".hidden", "ignored"] {
        std::fs::create_dir(directory.path().join(folder)).unwrap();
        std::fs::write(directory.path().join(folder).join("entry.txt"), b"plain file").unwrap();
    }
    let root = fwd(directory.path());
    let endpoint = Endpoint::new(&root, false);
    let mut builder = globset::GlobSetBuilder::new();
    builder.add(globset::Glob::new("ignored").unwrap());
    let globs = builder.build().unwrap();
    let tree = snapshot_agent::walk_hashed_via_agent(&endpoint.backend, &root,
        &AtomicBool::new(false), &WalkFilter::basic(false, &globs), HashMode::FullFresh)
        .unwrap().expect("regular tree retains the agent fast path");
    assert_eq!(tree.keys().map(String::as_str).collect::<Vec<_>>(), ["node_modules/entry.txt"]);
    assert_ne!(tree["node_modules/entry.txt"].hash, 0);
}

#[test]
fn sync_links_task_legacy_peer_uses_metadata_instead_of_silently_incomplete_hashes() {
    use crate::agent_proto::{self, Frame};
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    link_fixture::directory(outside.path(), &directory.path().join("node_modules"));
    std::fs::write(directory.path().join("normal.txt"), b"legacy peer").unwrap();
    let root = fwd(directory.path());
    let endpoint = Endpoint::connect(&root, |mut reader, mut writer, _| {
        while let Ok(Some((id, frame))) = agent_proto::read_frame(&mut reader) {
            let reply = match frame {
                Frame::Hello { .. } => Frame::HelloOk { proto: agent_proto::PROTO_VERSION,
                    version: "0.5.161 worker".into() },
                Frame::Stat(path) => Frame::Meta(agent_proto::stat_local(&path).unwrap()),
                Frame::ListDir(path) => Frame::Dir(agent_proto::list_local(&path).unwrap()),
                other => panic!("legacy peer must not receive a hash request: {other:?}"),
            };
            if agent_proto::write_frame(&mut writer, id, &reply).is_err() { break; }
        }
    });
    let globs = empty_globset();
    let snapshot = snapshot::walk_snapshot(&endpoint.backend, &root, &AtomicBool::new(false),
        &WalkFilter::basic(true, &globs), HashMode::None, None, false, true).unwrap();
    assert_eq!(snapshot.tree.keys().map(String::as_str).collect::<Vec<_>>(), ["normal.txt"]);
    assert_eq!(snapshot.omissions.reported_paths().collect::<Vec<_>>(), ["node_modules"]);
    assert!(agent_proto::has_link_aware_hash(agent_proto::HASH_WALK_SERVER_VERSION));
    assert!(!agent_proto::has_link_aware_hash("0.5.161+sync-links-v10"));
    link_fixture::remove_directory(&directory.path().join("node_modules"));
}
