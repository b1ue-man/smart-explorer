//! Places for connections before their registration (R-SERVER-04).

use std::io::{ErrorKind, Read};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::{hold, lock, release_current, HandshakeSlots, SourceKey};

const WAIT: Duration = Duration::from_secs(5);

/// A connected pair: the client end the test reads, the server end the pool
/// closes.
fn pair(listener: &TcpListener) -> (TcpStream, TcpStream) {
    let client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    (client, server)
}

/// Whether the server end shut the connection down; an open one stays
/// silent until the read times out.
fn closed(client: &mut TcpStream) -> bool {
    client
        .set_read_timeout(Some(Duration::from_millis(300)))
        .expect("read timeout");
    let mut byte = [0_u8; 1];
    match client.read(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(error) => !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
    }
}

fn waiting(slots: &HandshakeSlots) -> usize {
    lock(&slots.registry).entries.len()
}

#[test]
fn a_full_scope_closes_its_oldest_waiting_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let slots = HandshakeSlots::new(3, 2, 8);
    let (source_a, source_b, source_c) = (
        SourceKey::Ipv4([192, 0, 2, 1]),
        SourceKey::Ipv4([192, 0, 2, 2]),
        SourceKey::Ipv4([192, 0, 2, 3]),
    );
    let (mut a1_client, a1_server) = pair(&listener);
    let (mut a2_client, a2_server) = pair(&listener);
    let (mut a3_client, a3_server) = pair(&listener);
    let (mut b1_client, b1_server) = pair(&listener);
    let (mut c1_client, c1_server) = pair(&listener);

    let first = slots.try_acquire(source_a, &a1_server).expect("place");
    let second = slots.try_acquire(source_a, &a2_server).expect("place");
    assert_eq!(waiting(&slots), 2);

    // The source is at its limit of two: its oldest connection makes room.
    let third = slots.try_acquire(source_a, &a3_server).expect("place");
    assert!(closed(&mut a1_client), "the oldest of the source is closed");
    assert!(!closed(&mut a2_client));
    assert_eq!(waiting(&slots), 2);

    // Another source still finds a place: three in total.
    let fourth = slots.try_acquire(source_b, &b1_server).expect("place");
    assert_eq!(waiting(&slots), 3);
    assert!(!closed(&mut b1_client));

    // The whole pool is full: the oldest waiting connection of all makes room.
    let fifth = slots.try_acquire(source_c, &c1_server).expect("place");
    assert!(closed(&mut a2_client), "the oldest of the pool is closed");
    assert!(!closed(&mut a3_client));
    assert!(!closed(&mut b1_client));
    assert!(!closed(&mut c1_client));
    assert_eq!(waiting(&slots), 3);

    drop((first, second, third, fourth, fifth));
    assert_eq!(waiting(&slots), 0, "every place is given back");
}

#[test]
fn hold_and_release_current_free_the_place_of_the_connection_thread() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let slots = HandshakeSlots::new(8, 1, 8);
    let source = SourceKey::Ipv4([192, 0, 2, 1]);
    let (mut first_client, first_server) = pair(&listener);
    let slot = slots.try_acquire(source, &first_server).expect("place");

    let (held_sender, held) = mpsc::channel();
    let (go_sender, go) = mpsc::channel::<()>();
    let (done_sender, done) = mpsc::channel();
    let worker = thread::spawn(move || {
        hold(slot);
        held_sender.send(()).expect("held");
        go.recv_timeout(WAIT).expect("go");
        release_current();
        done_sender.send(()).expect("done");
    });
    held.recv_timeout(WAIT).expect("the thread holds the place");
    assert_eq!(waiting(&slots), 1);

    go_sender.send(()).expect("go");
    done.recv_timeout(WAIT).expect("the thread released");
    worker.join().expect("worker");
    assert_eq!(waiting(&slots), 0, "release_current gave the place back");

    // The place is free: a second connection of the same source (limit one)
    // closes nobody, and the first connection itself was not shut down.
    let (_second_client, second_server) = pair(&listener);
    let second = slots.try_acquire(source, &second_server).expect("place");
    assert!(!closed(&mut first_client));
    assert_eq!(waiting(&slots), 1);

    // A thread without a place (tests, direct handler calls) changes nothing.
    thread::spawn(release_current)
        .join()
        .expect("release without a place");
    assert_eq!(waiting(&slots), 1);
    drop(second);
    assert_eq!(waiting(&slots), 0);
}
