use super::super::*;
use super::fwd;
use crate::agent::AgentBackend;
use crate::vfs::{Backend, BackendHandle, LocalBackend};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{atomic::AtomicBool, Arc};

/// A remote client location cannot satisfy a framed request through local I/O.
struct RemoteClient(LocalBackend);

impl RemoteClient {
    fn unavailable<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "remote fixture must use its agent transport",
        ))
    }
}

impl Backend for RemoteClient {
    fn scheme(&self) -> crate::vfs::Scheme {
        self.0.scheme()
    }

    fn root_display(&self) -> String {
        self.0.root_display()
    }

    fn is_local(&self) -> bool {
        false
    }

    fn list_dir(&self, _path: &str) -> crate::vfs::VfsResult<Vec<crate::vfs::VfsMeta>> {
        Self::unavailable()
    }

    fn stat(&self, _path: &str) -> crate::vfs::VfsResult<crate::vfs::VfsMeta> {
        Self::unavailable()
    }

    fn open_read(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Read + Send>> {
        Self::unavailable()
    }

    fn open_write(&self, _path: &str) -> crate::vfs::VfsResult<Box<dyn Write + Send>> {
        Self::unavailable()
    }

    fn rename(&self, _source: &str, _destination: &str) -> crate::vfs::VfsResult<()> {
        Self::unavailable()
    }

    fn remove_file(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        Self::unavailable()
    }

    fn remove_dir(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        Self::unavailable()
    }

    fn mkdir_all(&self, _path: &str) -> crate::vfs::VfsResult<()> {
        Self::unavailable()
    }
}

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

    fn connect(
        root: &str,
        handler: impl FnOnce(TcpStream, TcpStream, BackendHandle) + Send + 'static,
    ) -> Self {
        Self::connect_client(root, false, handler)
    }

    fn remote(root: &str) -> Self {
        Self::connect_client(root, true, |reader, writer, _| {
            let _ = crate::agent_proto::serve(reader, writer);
        })
    }

    fn connect_client(
        root: &str,
        remote: bool,
        handler: impl FnOnce(TcpStream, TcpStream, BackendHandle) + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let served: BackendHandle = Arc::new(LocalBackend::new(root));
        let inner: BackendHandle = if remote {
            Arc::new(RemoteClient(LocalBackend::new(root)))
        } else {
            served.clone()
        };
        let server = std::thread::spawn(move || {
            let (writer, _) = listener.accept().unwrap();
            let reader = writer.try_clone().unwrap();
            handler(reader, writer, served);
        });
        let socket = TcpStream::connect(address).unwrap();
        let backend = AgentBackend::from_streams(
            Box::new(socket.try_clone().unwrap()),
            Box::new(socket.try_clone().unwrap()),
            inner,
        )
        .unwrap();
        Self {
            backend,
            socket,
            server: Some(server),
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
        if let Some(server) = self.server.take() {
            server.join().unwrap();
        }
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
        let error = endpoint
            .backend
            .walk_hashed(root, true, tx, &cancel)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            crate::agent_proto::HASH_WALK_LINK_BOUNDARY
        );
    }
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let opts = BisyncOptions {
        compare: CompareMode::Checksum,
        ..Default::default()
    };
    let result = super::super::run(&ea.backend, &ra, &eb.backend, &rb, opts, &cancel, &filter);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(result.conflicts.is_empty());
    assert_eq!(
        result.omissions.reported_paths().collect::<Vec<_>>(),
        ["node_modules", "target_link"]
    );
    assert_eq!(
        std::fs::read(b.path().join("from-a.txt")).unwrap(),
        b"agent source"
    );
    assert_eq!(
        std::fs::read(a.path().join("from-b.txt")).unwrap(),
        b"daemon source"
    );
    assert_eq!(
        std::fs::read(b.path().join("node_modules/keep.txt")).unwrap(),
        b"protected"
    );
    assert!(!b.path().join("node_modules/private.txt").exists());
    assert!(!outside.path().join("source.txt").exists());
    assert!(result
        .baseline
        .keys()
        .all(|path| !path.starts_with("node_modules/") && !path.starts_with("target_link/")));
    std::fs::remove_file(baseline_file(result.state.as_ref().unwrap()).unwrap()).unwrap();
    link_fixture::remove_directory(&a.path().join("node_modules"));
    link_fixture::remove_directory(&b.path().join("target_link"));
}

#[test]
fn sync_links_task_regular_agent_tree_keeps_fast_hash_path_and_filters() {
    let directory = tempfile::tempdir().unwrap();
    for folder in ["node_modules", ".hidden", "ignored"] {
        std::fs::create_dir(directory.path().join(folder)).unwrap();
        std::fs::write(
            directory.path().join(folder).join("entry.txt"),
            b"plain file",
        )
        .unwrap();
    }
    let root = fwd(directory.path());
    let endpoint = Endpoint::remote(&root);
    assert!(!endpoint.backend.is_local());
    let mut builder = globset::GlobSetBuilder::new();
    builder.add(globset::Glob::new("ignored").unwrap());
    builder.add(globset::Glob::new(".hidden/**").unwrap());
    let globs = builder.build().unwrap();
    // Hidden filtering rides on the agent's listing attributes (`walk-meta-v1`).
    let hidden_out = snapshot_agent::walk_hashed_via_agent(
        &endpoint.backend,
        &root,
        &AtomicBool::new(false),
        &WalkFilter::basic(false, &globs),
        HashMode::FullFresh,
    )
    .unwrap()
    .expect("an agent with listing attributes keeps the fast path");
    assert_eq!(
        hidden_out.keys().map(String::as_str).collect::<Vec<_>>(),
        ["node_modules/entry.txt"]
    );
    let tree = snapshot_agent::walk_hashed_via_agent(
        &endpoint.backend,
        &root,
        &AtomicBool::new(false),
        &WalkFilter::basic(true, &globs),
        HashMode::FullFresh,
    )
    .unwrap()
    .expect("regular tree retains the agent fast path");
    assert_eq!(
        tree.keys().map(String::as_str).collect::<Vec<_>>(),
        ["node_modules/entry.txt"]
    );
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
                Frame::Hello { .. } => Frame::HelloOk {
                    proto: agent_proto::PROTO_VERSION,
                    version: "0.5.161 worker".into(),
                },
                Frame::Stat(path) => Frame::Meta(agent_proto::stat_local(&path).unwrap()),
                Frame::ListDir(path) => Frame::Dir(agent_proto::list_local(&path).unwrap()),
                other => panic!("legacy peer must not receive a hash request: {other:?}"),
            };
            if agent_proto::write_frame(&mut writer, id, &reply).is_err() {
                break;
            }
        }
    });
    let globs = empty_globset();
    let snapshot = snapshot::walk_snapshot(
        &endpoint.backend,
        &root,
        &AtomicBool::new(false),
        &WalkFilter::basic(true, &globs),
        HashMode::None,
        None,
        false,
        true,
    )
    .unwrap();
    assert_eq!(
        snapshot.tree.keys().map(String::as_str).collect::<Vec<_>>(),
        ["normal.txt"]
    );
    assert_eq!(
        snapshot.omissions.reported_paths().collect::<Vec<_>>(),
        ["node_modules"]
    );
    assert!(agent_proto::has_link_aware_hash(
        agent_proto::HASH_WALK_SERVER_VERSION
    ));
    assert!(!agent_proto::has_link_aware_hash("0.5.161+sync-links-v10"));
    link_fixture::remove_directory(&directory.path().join("node_modules"));
}

/// Tree, folders, filtered files and omissions of one side.
#[cfg(unix)]
type SideFacts = (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<(String, super::super::OmissionKind, bool)>,
);

#[cfg(unix)]
fn side_facts(snapshot: snapshot::Snapshot) -> SideFacts {
    (
        snapshot.tree.keys().cloned().collect(),
        snapshot.dirs.into_iter().collect(),
        snapshot.filtered.keys().cloned().collect(),
        snapshot
            .omissions
            .paths()
            .map(|(path, kind, reported)| (path.to_string(), kind, reported))
            .collect(),
    )
}

// Unix: there both sides mark dot names hidden (Windows local listings
// use the hidden attribute instead).
#[cfg(unix)]
#[test]
fn abgleich_task_remote_host_walk_snapshot_equals_the_folder_walk() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    for (file, bytes) in [
        ("keep/a.txt", 3usize),
        ("keep/big.bin", 2000),
        (".dot/x.txt", 1),
        (".hidden.txt", 1),
        ("skip/y.txt", 1),
        ("sub/deep/z.txt", 4),
        ("node_modules/m.txt", 2),
    ] {
        let target = path.join(file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, vec![b'x'; bytes]).unwrap();
    }
    link_fixture::directory(&path.join("keep"), &path.join("link"));
    let fifo = std::ffi::CString::new(path.join("sub/fifo").to_string_lossy().as_bytes()).unwrap();
    // SAFETY: a valid NUL-terminated path; creates one FIFO node.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let root = fwd(path);
    let endpoint = Endpoint::remote(&root);
    let local = LocalBackend::new(&root);
    let mut builder = globset::GlobSetBuilder::new();
    builder.add(globset::Glob::new("skip").unwrap());
    let globs = builder.build().unwrap();
    let cancel = AtomicBool::new(false);
    for include_hidden in [false, true] {
        let filter = WalkFilter {
            max_size: 1000,
            ..WalkFilter::basic(include_hidden, &globs)
        };
        // The default job: no walk across mounts.
        let opts = BisyncOptions::default();
        assert!(!opts.cross_mounts);
        let read = |backend: &dyn Backend| {
            side_facts(
                snapshot::walk_snapshot_with_options(
                    backend,
                    &root,
                    &cancel,
                    &filter,
                    HashMode::None,
                    None,
                    false,
                    false,
                    opts,
                )
                .unwrap(),
            )
        };
        let folder_walk = read(&local);
        let host_walk = read(&endpoint.backend);
        assert_eq!(host_walk, folder_walk, "include_hidden = {include_hidden}");
        assert!(folder_walk.0.contains(&"keep/a.txt".to_string()));
        assert!(folder_walk.2.contains(&"keep/big.bin".to_string()));
        // The agent served this side with one walk next to the data.
        let tree = snapshot_agent::walk_hashed_via_agent(
            &endpoint.backend,
            &root,
            &cancel,
            &filter,
            HashMode::None,
        )
        .unwrap()
        .expect("one walk next to the data");
        assert_eq!(tree.keys().cloned().collect::<Vec<_>>(), folder_walk.0);
    }
    link_fixture::remove_directory(&path.join("link"));
}
