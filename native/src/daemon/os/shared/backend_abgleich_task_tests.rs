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
    let status = backend.stat(&format!("{root}/.versteckt")).unwrap();
    assert_eq!(status.hidden, expected);
    assert_eq!(status.size, 1);
    // No keyed walk behind the service: unsupported, nothing sent.
    let (tx, rx) = crossbeam_channel::unbounded();
    let request = HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta: true,
    };
    let ran =
        vfs::hash_walk_keyed(&backend, &root, request, &[], tx, &AtomicBool::new(false)).unwrap();
    assert!(!ran);
    assert_eq!(rx.try_iter().count(), 0);
    drop(backend);
    let _ = shutdown.shutdown(Shutdown::Both);
    server.join().unwrap();
}
