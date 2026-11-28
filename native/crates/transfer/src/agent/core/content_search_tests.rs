//! `AgentBackend::content_search` against the agent's request loop: the label
//! is announced, the events arrive in the caller's channel and the totals end
//! the request; `vfs::content_search` takes the agent's answer.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::super::AgentBackend;
use crate::agent_proto::{ContentEvent, ContentQuery};
use crate::vfs::Backend;

#[test]
fn content_search_task_agent_backend_searches_next_to_the_data() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("a.txt"), "one\nNeedle two\n").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/b.txt"), "needle\n").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let _ = crate::agent_proto::serve(read, socket);
    });
    let client = TcpStream::connect(address).unwrap();
    let shutdown = client.try_clone().unwrap();
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(client);
    let inner: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
    let backend = AgentBackend::from_streams(read, write, inner).unwrap();
    assert!(backend.supports_content_search());

    let root = dir.path().to_string_lossy().into_owned();
    let (tx, rx) = crossbeam_channel::unbounded();
    let summary = crate::vfs::content_search(
        &backend,
        &root,
        &ContentQuery::text("needle"),
        tx,
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut files: Vec<String> = rx
        .try_iter()
        .filter_map(|event| match event {
            ContentEvent::Hits { rel, .. } => Some(rel),
            _ => None,
        })
        .collect();
    files.sort();
    assert_eq!(files, ["a.txt", "sub/b.txt"]);
    assert_eq!(summary.files_matched, 2);
    assert_eq!(summary.hits, 2);

    // A canceled search ends with `Interrupted`.
    let (tx, _rx) = crossbeam_channel::unbounded();
    let error = backend
        .content_search(
            &root,
            &ContentQuery::text("needle"),
            tx,
            &AtomicBool::new(true),
        )
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);

    let _ = shutdown.shutdown(std::net::Shutdown::Both);
    drop(backend);
    server.join().unwrap();
}
