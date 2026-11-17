//! A long vault request waits as long as the worker that took it lives
//! (user rule 2026-11-17): only a replaced worker, a worker that stopped
//! answering or a closed connection ends the wait, never its duration.
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use super::{first_byte, Look, Stopped, WorkerWatch};

#[test]
fn zeitlimit_task_a_vault_wait_ends_only_when_the_worker_is_gone() {
    // Hours of looks at the same worker never end the wait.
    let mut watch = WorkerWatch::new(None);
    for _ in 0..1000 {
        assert_eq!(watch.look(Some("generation-a")), Look::Wait);
    }
    // A worker that misses an answer now and then is still there.
    for _ in 0..10 {
        assert_eq!(watch.look(None), Look::Wait);
        assert_eq!(watch.look(None), Look::Wait);
        assert_eq!(watch.look(Some("generation-a")), Look::Wait);
    }
    // Three looks in a row without an answer end it.
    assert_eq!(watch.look(None), Look::Wait);
    assert_eq!(watch.look(None), Look::Wait);
    assert_eq!(watch.look(None), Look::Silent);
    assert_eq!(WorkerWatch::MAX_SILENT, 3);

    // Another worker took over: the request went with the one before.
    let mut watch = WorkerWatch::new(Some("generation-a".into()));
    assert_eq!(watch.look(Some("generation-a")), Look::Wait);
    assert_eq!(watch.look(Some("generation-b")), Look::Replaced);
}

#[test]
fn zeitlimit_task_a_vault_wait_takes_the_answer_or_notices_the_closed_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    let address = listener.local_addr().expect("its address");
    let server = std::thread::spawn(move || {
        let (mut answering, _) = listener.accept().expect("the first client");
        std::thread::sleep(Duration::from_millis(300));
        answering.write_all(b"{}\n").expect("the answer");
        let (closing, _) = listener.accept().expect("the second client");
        std::thread::sleep(Duration::from_millis(300));
        drop(closing);
        answering
    });
    let client = TcpStream::connect(address).expect("the first connection");
    first_byte(&client).expect("the answer arrives");
    let mut byte = [0u8; 1];
    assert_eq!(client.peek(&mut byte).expect("the byte is still there"), 1);
    let closed = TcpStream::connect(address).expect("the second connection");
    assert!(matches!(first_byte(&closed), Err(Stopped::Closed)));
    drop(server.join().expect("the server"));
}
