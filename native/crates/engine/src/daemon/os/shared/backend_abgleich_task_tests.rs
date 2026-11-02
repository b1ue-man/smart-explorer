//! The background service of this build (G5, E10): `list_dir` and `stat`
//! through it carry the served backend's hidden flag (`meta-list-v1`), and
//! a served backend without a keyed walk answers `WalkKeyed` as unsupported
//! before any item, so the client takes the walk without keys.
use crate::vfs::{self as vfs, Backend, BackendHandle, HashWalkRequest, LocalBackend};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[test]
fn abgleich_task_service_listing_carries_the_served_hidden_flag() {
    let base = tempfile::tempdir().unwrap();
    std::fs::write(base.path().join(".versteckt"), b"h").unwrap();
    std::fs::write(base.path().join("sichtbar"), b"s").unwrap();
    let root = base.path().to_string_lossy().replace('\\', "/");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let served = root.clone();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let backend: BackendHandle = Arc::new(LocalBackend::new(&served));
        let _ = super::super::backend_server::serve_backend(read, socket, backend);
    });
    let client = TcpStream::connect(address).unwrap();
    let shutdown = client.try_clone().unwrap();
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(client);
    let inner: BackendHandle = Arc::new(LocalBackend::new(&root));
    let backend = crate::agent::AgentBackend::from_streams(read, write, inner).unwrap();
    let served_listing = LocalBackend::new(&root).list_dir(&root).unwrap();
    let expected = served_listing
        .iter()
        .find(|meta| meta.name == ".versteckt")
        .unwrap()
        .hidden;
    let listed = backend.list_dir(&root).unwrap();
    let hidden = listed
        .iter()
        .find(|meta| meta.name == ".versteckt")
        .unwrap();
    assert_eq!(hidden.hidden, expected, "as the served backend reports it");
    assert!(hidden.hidden, "a dot name is hidden on this side");
    assert!(
        !listed
            .iter()
            .find(|meta| meta.name == "sichtbar")
            .unwrap()
            .hidden
    );
    // `stat` carries what the served backend's own `stat` reports: on Linux
    // a local `stat` has no hidden attribute (`vfs/os/shared/local.rs`,
    // `meta_to_vfs` through `local_platform::local_attrs`), the dot-name rule
    // belongs to the listing (`local_access`); on Windows both read the
    // attribute.
    let path = format!("{root}/.versteckt");
    let served_status = LocalBackend::new(&root).stat(&path).unwrap();
    let status = backend.stat(&path).unwrap();
    assert_eq!(
        status.hidden, served_status.hidden,
        "as the served backend reports it"
    );
    assert_eq!(status.size, 1);
    // No keyed walk behind the service: unsupported, nothing sent.
    let (tx, rx) = crossbeam_channel::unbounded();
    let request = HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta: true,
        fresh: false,
        filter: None,
    };
    let ran = vfs::hash_walk_keyed(
        &backend,
        &root,
        request,
        vfs::KeyedWalk::KeysOnly,
        tx,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!ran);
    assert_eq!(rx.try_iter().count(), 0);
    drop(backend);
    let _ = shutdown.shutdown(Shutdown::Both);
    server.join().unwrap();
}

/// A walk of `walk` through `backend`, all items.
fn keyed(
    backend: &dyn Backend,
    root: &str,
    walk: vfs::KeyedWalk<'_>,
) -> (bool, Vec<vfs::KeyedWalkItem>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let request = HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta: true,
        fresh: false,
        filter: None,
    };
    let ran =
        vfs::hash_walk_keyed(backend, root, request, walk, tx, &AtomicBool::new(false)).unwrap();
    (ran, rx.try_iter().collect())
}

/// The service relays both key phases and the walk with known keys to a
/// served backend that walks next to its data (here an SSH agent of this
/// build), as the GUI's IPC path to the service does.
#[test]
fn abgleich_task_service_relays_the_key_phases() {
    let base = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        let dir = base.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), name).unwrap();
    }
    let root = base.path().to_string_lossy().replace('\\', "/");
    // The served backend: an agent next to the data.
    let agent_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let agent_address = agent_listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (socket, _) = agent_listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let _ = crate::agent_proto::serve(read, socket);
    });
    let to_agent = TcpStream::connect(agent_address).unwrap();
    let served: BackendHandle = Arc::new(
        crate::agent::AgentBackend::from_streams(
            Box::new(to_agent.try_clone().unwrap()),
            Box::new(to_agent),
            Arc::new(LocalBackend::new("/")),
        )
        .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let _ = super::super::backend_server::serve_backend(read, socket, served);
    });
    let client = TcpStream::connect(address).unwrap();
    let service = crate::agent::AgentBackend::from_streams(
        Box::new(client.try_clone().unwrap()),
        Box::new(client),
        Arc::new(LocalBackend::new("/")),
    )
    .unwrap();
    // Phase 1: keys only, one per folder, no item.
    let (ran, keys) = keyed(&service, &root, vfs::KeyedWalk::KeysOnly);
    assert!(ran);
    let named: Vec<(String, [u8; 32])> = keys
        .iter()
        .map(|item| match item {
            vfs::KeyedWalkItem::Key { rel, key, .. } => (rel.clone(), *key),
            other => panic!("keys only, got {other:?}"),
        })
        .collect();
    assert_eq!(named.len(), 3, "root, a and b");
    // Each key carries what its subtree holds, relayed unchanged.
    let tally = keys
        .iter()
        .find_map(|item| match item {
            vfs::KeyedWalkItem::Key { rel, tally, .. } if rel.is_empty() => *tally,
            _ => None,
        })
        .expect("the root's tally");
    assert_eq!((tally.files, tally.bytes, tally.height), (2, 2, 1));
    assert!(tally.clean());
    // Phase 2: the run of `a` alone.
    let folders = vec!["a".to_string()];
    let (ran, runs) = keyed(&service, &root, vfs::KeyedWalk::Runs { folders: &folders });
    assert!(ran);
    assert!(matches!(&runs[0], vfs::KeyedWalkItem::Run { rel } if rel == "a"));
    assert_eq!(runs.len(), 2, "its mark and f.txt: {runs:?}");
    // With every key known: one mark.
    let known: Vec<[u8; 16]> = named
        .iter()
        .map(|(_, key)| crate::agent_proto::known_prefix(key))
        .collect();
    let (ran, marks) = keyed(&service, &root, vfs::KeyedWalk::Marks { known: &known });
    assert!(ran);
    assert_eq!(marks.len(), 1, "{marks:?}");
}
