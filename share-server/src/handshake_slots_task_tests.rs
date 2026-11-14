//! Reserved places of registering connections (O-S5): a reserved place is not
//! evicted, a newcomer is refused by a scope whose places are all reserved, and
//! an evicted client reconnects successfully.

use std::io::{ErrorKind, Read};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::{hold, lock, release_current, reserve_current, HandshakeSlots, SourceKey};

const WAIT: Duration = Duration::from_secs(5);

/// A connected pair: the client end the test reads, the server end the pool
/// closes.
fn pair(listener: &TcpListener) -> (TcpStream, TcpStream) {
    let client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    (client, server)
}

/// Whether the server end shut the connection down; an open one stays silent
/// until the read times out.
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
fn server_rand_task_reserved_place_is_not_evicted_by_a_newcomer() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let slots = HandshakeSlots::new(8, 2, 8);
    let source = SourceKey::Ipv4([192, 0, 2, 1]);
    let (mut first_client, first_server) = pair(&listener);
    let (mut second_client, second_server) = pair(&listener);
    let (_third_client, third_server) = pair(&listener);

    let first = slots.try_acquire(source, &first_server).expect("place");
    let second = slots.try_acquire(source, &second_server).expect("place");
    // The first connection registers: its place is reserved.
    first.reserve();

    // The source is at its limit of two. The reserved place is skipped, and the
    // oldest waiting connection makes room.
    let third = slots.try_acquire(source, &third_server).expect("place");
    assert!(
        !closed(&mut first_client),
        "a reserved place is not evicted"
    );
    assert!(
        closed(&mut second_client),
        "the oldest waiting connection is evicted"
    );
    assert_eq!(waiting(&slots), 2);
    drop((first, second, third));
    assert_eq!(waiting(&slots), 0, "every place is given back");
}

#[test]
fn server_rand_task_a_scope_of_reserved_places_refuses_the_newcomer() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let slots = HandshakeSlots::new(8, 1, 8);
    let source = SourceKey::Ipv4([192, 0, 2, 1]);
    let (mut first_client, first_server) = pair(&listener);
    let (_second_client, second_server) = pair(&listener);

    let first = slots.try_acquire(source, &first_server).expect("place");
    first.reserve();

    // Nothing of the scope is evictable: the newcomer is refused, and
    // nothing is closed.
    assert!(slots.try_acquire(source, &second_server).is_none());
    assert!(!closed(&mut first_client));
    assert_eq!(waiting(&slots), 1);

    // Once the registration is over, the place is free again.
    drop(first);
    assert!(slots.try_acquire(source, &second_server).is_some());
}

#[test]
fn server_rand_task_an_evicted_client_reconnects_and_gets_a_place() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let slots = HandshakeSlots::new(8, 1, 8);
    let source = SourceKey::Ipv4([192, 0, 2, 1]);
    let (mut first_client, first_server) = pair(&listener);
    let (mut second_client, second_server) = pair(&listener);
    let (_reconnect_client, reconnect_server) = pair(&listener);

    let first = slots.try_acquire(source, &first_server).expect("place");
    let second = slots.try_acquire(source, &second_server).expect("place");
    assert!(closed(&mut first_client), "the first connection is evicted");
    drop(first);

    // The evicted client reconnects: it gets a place, and the newer waiting
    // connection of its scope makes room.
    let reconnect = slots
        .try_acquire(source, &reconnect_server)
        .expect("the reconnect gets a place");
    assert!(
        closed(&mut second_client),
        "the newer waiting connection makes room"
    );
    assert_eq!(waiting(&slots), 1);
    drop((second, reconnect));
    assert_eq!(waiting(&slots), 0);
}

#[test]
fn server_rand_task_reserve_current_keeps_the_thread_place_until_release() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let slots = HandshakeSlots::new(8, 1, 8);
    let source = SourceKey::Ipv4([192, 0, 2, 1]);
    let (mut first_client, first_server) = pair(&listener);
    let (_second_client, second_server) = pair(&listener);
    let slot = slots.try_acquire(source, &first_server).expect("place");

    let (reserved_sender, reserved) = mpsc::channel();
    let (go_sender, go) = mpsc::channel::<()>();
    let (done_sender, done) = mpsc::channel();
    let worker = thread::spawn(move || {
        hold(slot);
        reserve_current();
        reserved_sender.send(()).expect("reserved");
        go.recv_timeout(WAIT).expect("go");
        release_current();
        done_sender.send(()).expect("done");
    });
    reserved
        .recv_timeout(WAIT)
        .expect("the thread reserved its place");

    // While the place is reserved, a newcomer of the same source is refused and
    // the registering connection is not closed.
    assert!(slots.try_acquire(source, &second_server).is_none());
    assert!(!closed(&mut first_client));

    go_sender.send(()).expect("go");
    done.recv_timeout(WAIT)
        .expect("the thread released its place");
    worker.join().expect("worker");
    assert_eq!(waiting(&slots), 0, "release_current gave the place back");
    assert!(slots.try_acquire(source, &second_server).is_some());
}
