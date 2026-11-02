use super::{prepare_ipc_client_stream, read_pre_auth_line, remaining_until, PreAuthLimiter};
use std::io::{self, Read};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

#[cfg(any(target_os = "linux", target_os = "android"))]
fn spawn_accept_loop(
    wait: Duration,
    external: std::sync::Arc<std::sync::atomic::AtomicBool>,
    checks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> (
    super::IpcListener,
    std::sync::mpsc::Receiver<std::net::SocketAddr>,
    std::thread::JoinHandle<()>,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let stopping = Arc::new(AtomicBool::new(false));
    let loop_stopping = Arc::clone(&stopping);
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let external_stop = || {
            checks.fetch_add(1, Ordering::SeqCst);
            external.load(Ordering::SeqCst)
        };
        super::accept_loop(
            &listener,
            wait,
            &loop_stopping,
            &external_stop,
            |stream, peer| {
                drop(stream);
                let _ = sender.send(peer);
            },
            || false,
        );
    });
    (super::IpcListener { addr, stopping }, receiver, thread)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn android_background_task_ipc_listener_blocks_and_stops_on_shutdown() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    let checks = Arc::new(AtomicUsize::new(0));
    let external = Arc::new(AtomicBool::new(false));
    let (handle, admitted, thread) =
        spawn_accept_loop(Duration::from_secs(5), external, Arc::clone(&checks));

    let client = TcpStream::connect(handle.addr).unwrap();
    let peer = admitted.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(peer, client.local_addr().unwrap());

    // Idle: the thread blocks in the wait instead of a 100 ms poll.
    std::thread::sleep(Duration::from_millis(100));
    let before = checks.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(700));
    let idle_checks = checks.load(Ordering::SeqCst) - before;
    assert!(
        idle_checks <= 1,
        "listener woke {idle_checks} times while idle"
    );

    // Shutdown wakes the blocking wait at once; the wake is not served.
    let started = Instant::now();
    drop(handle);
    thread.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(admitted.try_recv().is_err());
    drop(client);
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn android_background_task_ipc_listener_honours_stop_control_within_wait() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    let external = Arc::new(AtomicBool::new(false));
    let (handle, admitted, thread) = spawn_accept_loop(
        Duration::from_millis(200),
        Arc::clone(&external),
        Arc::new(AtomicUsize::new(0)),
    );
    // A stop control written by another process (or a handoff to a newer
    // generation) is seen at the next wake without any connection.
    let started = Instant::now();
    external.store(true, Ordering::SeqCst);
    let (done_sender, done) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        thread.join().unwrap();
        let _ = done_sender.send(());
    });
    done.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(admitted.try_recv().is_err());
    // The loop already ended; dropping the handle only wakes a closed port.
    drop(handle);
}

#[test]
fn pre_auth_limit_releases_capacity_with_the_permit() {
    let limiter = PreAuthLimiter::new(2);
    let first = limiter.try_acquire().unwrap();
    let second = limiter.try_acquire().unwrap();
    assert!(limiter.try_acquire().is_none());

    drop(first);
    let replacement = limiter.try_acquire().unwrap();
    assert!(limiter.try_acquire().is_none());

    drop(second);
    drop(replacement);
    assert!(limiter.try_acquire().is_some());
}

#[test]
fn absolute_deadline_never_refreshes_after_expiry() {
    let now = Instant::now();
    let deadline = now + Duration::from_secs(5);
    assert_eq!(
        remaining_until(deadline, now).unwrap(),
        Duration::from_secs(5)
    );
    assert_eq!(
        remaining_until(deadline, deadline).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(
        remaining_until(deadline, deadline + Duration::from_millis(1))
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn incomplete_pre_auth_read_times_out_and_releases_capacity() {
    let limiter = PreAuthLimiter::new(1);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut server, _) = listener.accept().unwrap();

    let error = {
        let _permit = limiter.try_acquire().unwrap();
        assert!(limiter.try_acquire().is_none());
        let mut line = String::new();
        read_pre_auth_line(
            &mut server,
            &mut line,
            Instant::now() + Duration::from_millis(80),
        )
        .unwrap_err()
    };

    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(limiter.try_acquire().is_some());
    drop(client);
}

#[test]
fn accepted_ipc_stream_is_forced_back_to_blocking() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).unwrap();
    let (mut server, _) = loop {
        match listener.accept() {
            Ok(pair) => break pair,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept failed: {error}"),
        }
    };
    server.set_nonblocking(true).unwrap();
    prepare_ipc_client_stream(&server).unwrap();
    server
        .set_read_timeout(Some(Duration::from_millis(120)))
        .unwrap();

    let started = Instant::now();
    let mut one = [0u8; 1];
    let error = server.read(&mut one).unwrap_err();
    assert!(matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ));
    assert!(
        started.elapsed() >= Duration::from_millis(40),
        "read returned immediately; stream is still nonblocking"
    );
    drop(client);
}
