use super::{cancel_request, canceled_request_lost_client, serve_backend, transfer_channel};
use crate::agent_proto::{write_frame, Frame, TRANSFER_FRAME_BACKLOG};
use crate::vfs::Backend;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TrySendError;
use std::sync::{Arc, Mutex};

#[test]
fn inbound_transfer_channel_is_bounded_and_disconnects_both_ends() {
    let (sender, receiver) = transfer_channel();
    for _ in 0..TRANSFER_FRAME_BACKLOG {
        sender.try_send(Frame::Ok).unwrap();
    }
    assert!(matches!(
        sender.try_send(Frame::Ok),
        Err(TrySendError::Full(Frame::Ok))
    ));
    drop(receiver);
    assert!(matches!(
        sender.try_send(Frame::Ok),
        Err(TrySendError::Disconnected(Frame::Ok))
    ));
    let (sender, receiver) = transfer_channel();
    drop(sender);
    assert!(receiver.recv().is_err());
}

#[test]
fn cancel_disconnects_a_blocked_transfer_receiver() {
    let (sender, receiver) = transfer_channel();
    let inbound = Mutex::new(HashMap::from([(9, sender)]));
    let cancel = Arc::new(AtomicBool::new(false));
    let cancels = Mutex::new(HashMap::from([(9, cancel.clone())]));
    cancel_request(9, &inbound, &cancels);
    assert!(cancel.load(Ordering::Relaxed));
    assert!(receiver.recv().is_err());
}

#[test]
fn only_canceled_requests_treat_lost_error_reporting_as_terminal() {
    let canceled = std::io::Error::new(std::io::ErrorKind::Interrupted, "canceled");
    let backend_failure = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    let disconnected = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "peer closed");
    let reporting_failure = std::io::Error::other("writer lock poisoned");

    assert!(canceled_request_lost_client(&canceled, &disconnected, true));
    assert!(!canceled_request_lost_client(
        &backend_failure,
        &disconnected,
        true
    ));
    assert!(!canceled_request_lost_client(
        &canceled,
        &reporting_failure,
        true
    ));
    assert!(!canceled_request_lost_client(
        &canceled,
        &disconnected,
        false
    ));
}

#[test]
fn backend_server_proxies_bulk_folder_transfer() {
    let base = std::env::temp_dir().join(format!(
        "se_daemon_backend_{}_{}",
        std::process::id(),
        crate::share::core_now_secs()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("src/sub")).unwrap();
    std::fs::write(base.join("src/a.txt"), b"alpha").unwrap();
    std::fs::write(base.join("src/sub/b.txt"), b"bravo").unwrap();
    let root = base.to_string_lossy().replace('\\', "/");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server_root = root.clone();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let backend: crate::vfs::BackendHandle =
            Arc::new(crate::vfs::LocalBackend::new(&server_root));
        serve_backend(read, socket, backend).unwrap();
    });

    let client = TcpStream::connect(address).unwrap();
    let shutdown = client.try_clone().unwrap();
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(client);
    let inner: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new(&root));
    let backend = crate::agent::AgentBackend::from_streams(read, write, inner).unwrap();
    assert!(backend.supports_bulk_tree());
    let destination = format!("{root}/uploaded");
    std::fs::create_dir_all(base.join("uploaded")).unwrap();
    std::fs::write(base.join("uploaded/a.txt"), b"old destination").unwrap();
    assert_eq!(
        backend.put_tree(&base.join("src"), &destination).unwrap(),
        2
    );
    assert_eq!(
        std::fs::read(base.join("uploaded/sub/b.txt")).unwrap(),
        b"bravo"
    );
    let output = base.join("downloaded");
    assert_eq!(backend.get_tree(&destination, &output).unwrap(), 2);
    assert_eq!(std::fs::read(output.join("a.txt")).unwrap(), b"alpha");
    drop(backend);
    let _ = shutdown.shutdown(Shutdown::Both);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn socket_disconnect_cancels_backend_put_tree_and_preserves_destination() {
    let root = std::env::temp_dir().join(format!(
        "se_daemon_server_disconnect_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let destination = root.join("file.txt");
    std::fs::write(&destination, b"old").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let backend: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
        serve_backend(read, socket, backend).unwrap();
    });
    let mut client = TcpStream::connect(address).unwrap();
    write_frame(
        &mut client,
        8,
        &Frame::PutTree(root.to_string_lossy().into_owned()),
    )
    .unwrap();
    write_frame(
        &mut client,
        8,
        &Frame::TreeEntry {
            rel: "file.txt".into(),
            is_dir: false,
            size: 3,
            mtime_ms: 0,
        },
    )
    .unwrap();
    write_frame(&mut client, 8, &Frame::Data(b"new".to_vec())).unwrap();
    client.shutdown(Shutdown::Both).unwrap();
    drop(client);
    server.join().unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
    assert!(!std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .contains(".se-daemon-tree-")));
    let _ = std::fs::remove_dir_all(root);
}
