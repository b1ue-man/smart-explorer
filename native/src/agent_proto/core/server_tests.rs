use super::{join_workers, serve};
use crate::agent_proto::server_session::{transfer_channel, ServerSession};
use crate::agent_proto::{write_frame, Frame, TRANSFER_FRAME_BACKLOG};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    let session = ServerSession::new(Arc::new(Mutex::new(Box::new(Vec::<u8>::new()))));
    let context = session.open(9, &Frame::Write("/upload".into()));
    let inbound = context.inbound.expect("upload requests receive frames");

    session.cancel(9);

    assert!(context.cancel.load(Ordering::Relaxed));
    assert!(matches!(
        inbound.recv_timeout(Duration::from_secs(1)),
        Err(RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn socket_disconnect_cancels_put_tree_and_preserves_destination() {
    let root = std::env::temp_dir().join(format!(
        "se_agent_server_disconnect_{}_{}",
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
        let _ = serve(read, socket);
    });
    let mut client = TcpStream::connect(address).unwrap();
    write_frame(
        &mut client,
        7,
        &Frame::PutTree(root.to_string_lossy().into_owned()),
    )
    .unwrap();
    write_frame(
        &mut client,
        7,
        &Frame::TreeEntry {
            rel: "file.txt".into(),
            is_dir: false,
            size: 3,
            mtime_ms: 0,
        },
    )
    .unwrap();
    write_frame(&mut client, 7, &Frame::Data(b"new".to_vec())).unwrap();
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
            .contains(".se-agent-tree-")));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn worker_shutdown_joins_cleanup_before_returning() {
    let cleaned = Arc::new(AtomicBool::new(false));
    let cleaned_worker = cleaned.clone();
    let mut workers = vec![std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        cleaned_worker.store(true, Ordering::Relaxed);
    })];
    join_workers(&mut workers).unwrap();
    assert!(workers.is_empty());
    assert!(cleaned.load(Ordering::Relaxed));
}

#[test]
fn worker_shutdown_reports_panics() {
    let mut workers = vec![std::thread::spawn(|| panic!("request worker test panic"))];
    let error = join_workers(&mut workers).unwrap_err();
    assert!(error.to_string().contains("panicked"));
    assert!(workers.is_empty());
}
