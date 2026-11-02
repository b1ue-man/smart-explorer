//! The SSH agent of this build through the real server: its walk with folder
//! keys answers a known tree with one mark (E10), its listing and status
//! carry hidden dot names and creation times like its walk (`meta-list-v1`),
//! and an agent that cannot hash a file hands the digest to its SSH
//! connection instead of downloading the file (E9).
use super::AgentBackend;
use crate::agent_proto::{known_prefix, read_frame, write_frame, Frame, PROTO_VERSION};
use crate::vfs::{
    self as vfs, Backend, BackendHandle, HashWalkRequest, KeyedWalkItem, LocalBackend,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn connect(serve: impl FnOnce(TcpStream, TcpStream) + Send + 'static) -> AgentBackend {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        serve(read, socket);
    });
    let client = TcpStream::connect(address).unwrap();
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(client);
    let inner: BackendHandle = Arc::new(LocalBackend::new("/"));
    AgentBackend::from_streams(read, write, inner).unwrap()
}

fn real_agent() -> AgentBackend {
    connect(|read, write| {
        let _ = crate::agent_proto::serve(read, write);
    })
}

fn keyed(backend: &AgentBackend, root: &str, known: &[[u8; 16]]) -> (bool, Vec<KeyedWalkItem>) {
    let request = HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta: true,
        fresh: false,
        filter: None,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let ran = vfs::hash_walk_keyed(
        backend,
        root,
        request,
        vfs::KeyedWalk::Marks { known },
        tx,
        &AtomicBool::new(false),
    )
    .unwrap();
    (ran, rx.try_iter().collect())
}

#[test]
fn abgleich_task_agent_walk_with_keys_answers_a_known_tree_with_one_mark() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for folder in 0..4 {
        let dir = root.join(format!("d{folder}"));
        std::fs::create_dir_all(dir.join("leer")).unwrap();
        std::fs::write(dir.join("f.txt"), format!("{folder}")).unwrap();
    }
    let backend = real_agent();
    let root = root.to_str().unwrap();
    let (ran, first) = keyed(&backend, root, &[]);
    assert!(ran);
    let runs = first
        .iter()
        .filter(|item| matches!(item, KeyedWalkItem::Run { .. }))
        .count();
    assert_eq!(runs, 9, "the root, four folders and their empty subfolders");
    let known: Vec<[u8; 16]> = first
        .iter()
        .filter_map(|item| match item {
            KeyedWalkItem::Key {
                key, same: false, ..
            } => Some(known_prefix(key)),
            _ => None,
        })
        .collect();
    assert_eq!(known.len(), 9);
    let (ran, second) = keyed(&backend, root, &known);
    assert!(ran);
    assert_eq!(second.len(), 1, "{second:?}");
    assert!(matches!(&second[0], KeyedWalkItem::Key { rel, same: true, .. } if rel.is_empty()));
}

#[test]
fn abgleich_task_agent_listing_and_status_carry_hidden_and_creation_time() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join(".versteckt"), b"h").unwrap();
    std::fs::write(root.join("sichtbar.txt"), b"s").unwrap();
    let backend = real_agent();
    let listed = backend.list_dir(root.to_str().unwrap()).unwrap();
    let find = |name: &str| listed.iter().find(|meta| meta.name == name).unwrap();
    assert!(find(".versteckt").hidden);
    assert!(!find("sichtbar.txt").hidden);
    let path = root.join(".versteckt");
    let status = backend.stat(path.to_str().unwrap()).unwrap();
    assert!(status.hidden);
    assert_eq!(status.size, 1);
    if std::fs::metadata(&path).and_then(|m| m.created()).is_ok() {
        assert!(status.btime_ms > 0, "the file system keeps creation times");
        assert!(find(".versteckt").btime_ms > 0);
    }
    let tolerant = vfs::list_dir_tolerant(&backend, root.to_str().unwrap()).unwrap();
    assert!(tolerant
        .entries
        .iter()
        .any(|meta| meta.name == ".versteckt" && meta.hidden));
}

#[test]
fn abgleich_task_agent_without_file_digest_hashes_through_its_connection() {
    // A server without the extension frames answers no `FILE_DIGEST`: the
    // digest comes from the connection behind the agent (here the local
    // file system, standing in for the SSH server's hash command).
    let backend = connect(|mut read, mut write| {
        let Ok(Some((id, Frame::Hello { .. }))) = read_frame(&mut read) else {
            return;
        };
        let reply = Frame::HelloOk {
            proto: PROTO_VERSION,
            version: "0.1.0".into(),
        };
        let _ = write_frame(&mut write, id, &reply);
        while let Ok(Some(_)) = read_frame(&mut read) {}
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("datei.bin");
    std::fs::write(&path, b"abc").unwrap();
    let digest = vfs::content_digest(&backend, path.to_str().unwrap(), &AtomicBool::new(false))
        .unwrap()
        .expect("the connection behind the agent hashes the file");
    assert_eq!(
        digest.sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(digest.size, 3);
}

/// A keys-only walk through the real server carries each folder's tally.
#[test]
fn abgleich_task_agent_keys_only_walk_counts_each_subtree() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("top.txt"), b"12345").unwrap();
    std::fs::write(root.join("a/b/deep.txt"), b"123").unwrap();
    std::fs::create_dir_all(root.join("empty")).unwrap();
    let backend = real_agent();
    let request = HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta: true,
        fresh: false,
        filter: None,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let ran = vfs::hash_walk_keyed(
        &backend,
        root.to_str().unwrap(),
        request,
        vfs::KeyedWalk::KeysOnly,
        tx,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(ran);
    let mut tallies = std::collections::HashMap::new();
    for item in rx.try_iter() {
        match item {
            KeyedWalkItem::Key {
                rel,
                tally: Some(tally),
                ..
            } => {
                tallies.insert(rel, tally);
            }
            other => panic!("keys only sends tallied keys, got {other:?}"),
        }
    }
    assert_eq!(tallies.len(), 4, "root, a, a/b and empty: {tallies:?}");
    let top = tallies[""];
    assert_eq!(
        (top.files, top.bytes, top.height, top.clean()),
        (2, 8, 2, true)
    );
    assert_eq!((tallies["a"].files, tallies["a"].height), (1, 1));
    assert_eq!((tallies["empty"].files, tallies["empty"].height), (0, 0));
}
