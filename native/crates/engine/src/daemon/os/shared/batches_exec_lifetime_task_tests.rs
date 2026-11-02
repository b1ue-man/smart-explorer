//! B5 owner fixtures: real control sockets and the real ShareExecInput watch.
use super::*;
use std::{io::Write, sync::mpsc};

const WAIT: Duration = Duration::from_secs(10);

fn id(byte: &str) -> crate::share::ExecId {
    crate::share::ExecId::parse(byte.repeat(16)).unwrap()
}

fn sockets() -> (TcpStream, TcpStream) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect_timeout(&listener.local_addr().unwrap(), WAIT).unwrap();
    let (server, _) = listener.accept().unwrap();
    for socket in [&client, &server] {
        socket.set_read_timeout(Some(WAIT)).unwrap();
        socket.set_write_timeout(Some(WAIT)).unwrap();
    }
    (client, server)
}

fn control(
    owner: &Arc<ExecLifetime>,
    registry: Arc<ExecState>,
) -> (
    TcpStream,
    mpsc::Receiver<io::Result<()>>,
    std::thread::JoinHandle<()>,
) {
    let (mut client, server) = sockets();
    let token = owner.token().to_owned();
    let exec_id = owner.exec_id().clone();
    let (done, completed) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _ = done.send(serve_control(server, &registry, &exec_id, &token));
    });
    assert!(matches!(
        read_response(&mut client).unwrap(),
        IpcResponse::ExecControlReady
    ));
    // ACK bytes are written immediately before ready(). Mirror the production
    // handoff predicate so terminal cleanup does not race that final setup step.
    let deadline = Instant::now() + WAIT;
    let mut state = owner.state.lock().unwrap();
    while !state.ready {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("control owner reaches ready after its ACK");
        state = owner.changed.wait_timeout(state, remaining).unwrap().0;
        assert!(!state.cancelled && !state.finished);
    }
    drop(state);
    (client, completed, thread)
}

#[test]
fn batches_task_b5_control_owner_cancels_full_stdin_without_acquiring_stdin_mutex() {
    let mut fixture = crate::share::ShareExecInput::batches_fixture();
    fixture
        .input
        .send(crate::share::ExecClientInput::Stdin(vec![1]))
        .unwrap();
    let producer_input = fixture.input.clone();
    let (started, start_rx) = mpsc::channel();
    let (sent, sent_rx) = mpsc::channel();
    let producer = std::thread::spawn(move || {
        started.send(()).unwrap();
        let _ = sent.send(producer_input.send(crate::share::ExecClientInput::Stdin(vec![2])));
    });
    start_rx.recv_timeout(WAIT).unwrap();
    assert!(matches!(
        sent_rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    let owner = Arc::new(ExecLifetime::new(id("51"), fixture.input.clone()).unwrap());
    let second = ExecLifetime::new(id("51"), fixture.input.clone()).unwrap();
    assert_eq!(owner.token().len(), 64);
    assert_ne!(owner.token(), second.token());
    let registry = Arc::new(ExecState::new());
    registry.register_owner(owner.clone()).unwrap();
    let registration = Registration {
        owner: owner.clone(),
        registry: registry.clone(),
    };
    assert!(registry.owner(&id("52"), owner.token()).is_none());
    assert!(registry.owner(owner.exec_id(), second.token()).is_none());
    let (client, completed, worker) = control(&owner, registry.clone());
    let (_duplicate_client, duplicate_server) = sockets();
    assert_eq!(
        serve_control(duplicate_server, &registry, owner.exec_id(), owner.token())
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(
        !*fixture.cancel.borrow(),
        "duplicate control cannot cancel the legitimate owner"
    );

    let (data, _data_peer) = sockets();
    let input = super::super::ExecIpcInput {
        stream: Arc::new(Mutex::new(data)),
        control: Arc::new(client.try_clone().unwrap()),
    };
    let lock = input.stream.lock().unwrap();
    let cancel_input = input.clone();
    let (cancelled, cancel_rx) = mpsc::channel();
    let canceller = std::thread::spawn(move || {
        let _ = cancelled.send(cancel_input.cancel());
    });
    let result = cancel_rx.recv_timeout(WAIT);
    // Release a regressed mutex-dependent implementation before reporting it.
    drop(lock);
    let _ = client.shutdown(Shutdown::Both);
    result
        .expect("public cancel must finish while stdin mutex remains held")
        .unwrap();
    canceller.join().unwrap();
    completed
        .recv_timeout(WAIT)
        .expect("control reader observes cancellation")
        .unwrap();
    worker.join().unwrap();
    assert!(
        *fixture.cancel.borrow(),
        "real ShareExecInput uses its independent watch"
    );
    assert!(sent_rx
        .recv_timeout(WAIT)
        .expect("full stdin send wakes on cancellation")
        .is_err());
    producer.join().unwrap();
    assert_eq!(
        fixture.data.try_recv().unwrap(),
        crate::share::ExecClientInput::Stdin(vec![1])
    );
    assert!(matches!(
        fixture.data.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    drop(registration);
    assert!(registry.owner(owner.exec_id(), owner.token()).is_none());
}

#[test]
fn batches_task_b5_terminal_registration_closes_control_without_cancelling_finished_owner() {
    let fixture = crate::share::ShareExecInput::batches_fixture();
    let owner = Arc::new(ExecLifetime::new(id("53"), fixture.input.clone()).unwrap());
    let registry = Arc::new(ExecState::new());
    registry.register_owner(owner.clone()).unwrap();
    let registration = Registration {
        owner: owner.clone(),
        registry: registry.clone(),
    };
    let (mut client, completed, worker) = control(&owner, registry.clone());
    drop(registration);
    let mut byte = [0];
    assert_eq!(client.read(&mut byte).unwrap(), 0);
    completed.recv_timeout(WAIT).unwrap().unwrap();
    worker.join().unwrap();
    assert!(
        !*fixture.cancel.borrow(),
        "terminal cleanup must not issue a new Cancel"
    );
    assert!(registry.owner(owner.exec_id(), owner.token()).is_none());
    let (_client, server) = sockets();
    assert_eq!(
        owner.claim(&server).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn batches_task_b5_finished_owner_releases_control_reader_without_socket_shutdown() {
    let fixture = crate::share::ShareExecInput::batches_fixture();
    let owner = Arc::new(ExecLifetime::new(id("55"), fixture.input.clone()).unwrap());
    let registry = Arc::new(ExecState::new());
    registry.register_owner(owner.clone()).unwrap();
    let registration = Registration {
        owner: owner.clone(),
        registry: registry.clone(),
    };
    let (mut client, completed, worker) = control(&owner, registry.clone());
    // Keep both endpoints live, but deny finish its socket shutdown handle.
    // The real reader must observe the owner predicate and its notification.
    let shutdown_handle = owner.state.lock().unwrap().control.take().unwrap();
    drop(registration);
    let result = completed.recv_timeout(WAIT);
    if result.is_err() {
        let _ = client.shutdown(Shutdown::Both);
    }
    result
        .expect("finished owner releases the reader without a socket shutdown")
        .unwrap();
    worker.join().unwrap();
    let state = owner.state.lock().unwrap();
    assert!(state.finished && !state.cancelled);
    drop(state);
    assert!(!*fixture.cancel.borrow());
    assert!(registry.owner(owner.exec_id(), owner.token()).is_none());
    drop(shutdown_handle);
    assert_eq!(client.read(&mut [0]).unwrap(), 0);
}

#[test]
fn batches_task_b5_idle_control_owner_stays_live_until_peer_cancels() {
    let fixture = crate::share::ShareExecInput::batches_fixture();
    let owner = Arc::new(ExecLifetime::new(id("56"), fixture.input.clone()).unwrap());
    let registry = Arc::new(ExecState::new());
    registry.register_owner(owner.clone()).unwrap();
    let registration = Registration {
        owner: owner.clone(),
        registry: registry.clone(),
    };
    let (client, completed, worker) = control(&owner, registry.clone());
    assert!(matches!(
        completed.recv_timeout(OWNER_POLL_INTERVAL * 4),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(!*fixture.cancel.borrow(), "idle control is not cancellation");
    assert!(registry.owner(owner.exec_id(), owner.token()).is_some());
    drop(client);
    completed
        .recv_timeout(WAIT)
        .expect("peer EOF still ends its live owner")
        .unwrap();
    worker.join().unwrap();
    assert!(*fixture.cancel.borrow());
    drop(registration);
    assert!(registry.owner(owner.exec_id(), owner.token()).is_none());
}

#[test]
fn batches_task_b5_finished_control_cannot_cancel_same_id_live_owner() {
    let finished_fixture = crate::share::ShareExecInput::batches_fixture();
    let live_fixture = crate::share::ShareExecInput::batches_fixture();
    let finished = Arc::new(ExecLifetime::new(id("57"), finished_fixture.input.clone()).unwrap());
    let live = Arc::new(ExecLifetime::new(id("57"), live_fixture.input.clone()).unwrap());
    assert_ne!(finished.token(), live.token());
    let registry = Arc::new(ExecState::new());
    registry.register_owner(finished.clone()).unwrap();
    registry.register_owner(live.clone()).unwrap();
    let finished_registration = Registration {
        owner: finished.clone(),
        registry: registry.clone(),
    };
    let live_registration = Registration {
        owner: live.clone(),
        registry: registry.clone(),
    };
    let (mut finished_client, finished_rx, finished_worker) = control(&finished, registry.clone());
    let (mut live_client, live_rx, live_worker) = control(&live, registry.clone());
    drop(finished_registration);
    finished_rx.recv_timeout(WAIT).unwrap().unwrap();
    finished_worker.join().unwrap();
    assert_eq!(finished_client.read(&mut [0]).unwrap(), 0);
    assert!(registry.owner(finished.exec_id(), finished.token()).is_none());
    assert!(registry.owner(live.exec_id(), live.token()).is_some());
    assert!(matches!(
        live_rx.recv_timeout(OWNER_POLL_INTERVAL * 4),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(!*finished_fixture.cancel.borrow());
    assert!(!*live_fixture.cancel.borrow());
    let (_old_client, old_server) = sockets();
    assert_eq!(
        serve_control(old_server, &registry, finished.exec_id(), finished.token())
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(!*live_fixture.cancel.borrow());
    live_client.write_all(&[0x7f]).unwrap();
    live_rx
        .recv_timeout(WAIT)
        .expect("unexpected control data still cancels the bound live owner")
        .unwrap();
    live_worker.join().unwrap();
    assert!(*live_fixture.cancel.borrow());
    assert!(!*finished_fixture.cancel.borrow());
    drop(live_registration);
    assert!(registry.owner(live.exec_id(), live.token()).is_none());
}

#[test]
fn batches_task_b5_session_drop_releases_control_and_data_despite_input_clone_and_lock() {
    for finished in [false, true] {
        let (data, mut data_peer) = sockets();
        let events = data.try_clone().unwrap();
        let (control, mut control_peer) = sockets();
        let input = super::super::ExecIpcInput {
            stream: Arc::new(Mutex::new(data)),
            control: Arc::new(control),
        };
        let session = super::super::ExecIpcSession {
            exec_id: id("54"),
            input: input.clone(),
            events,
            finished,
        };
        let lock = input.stream.lock().unwrap();
        let (dropped, drop_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            drop(session);
            let _ = dropped.send(());
        });
        let result = drop_rx.recv_timeout(WAIT);
        drop(lock);
        result.expect("session drop must not acquire the held stdin lock");
        worker.join().unwrap();
        let mut byte = [0];
        if finished {
            for peer in [&mut control_peer, &mut data_peer] {
                peer.set_read_timeout(Some(Duration::from_millis(100)))
                    .unwrap();
                let error = peer
                    .read(&mut byte)
                    .expect_err("finished session leaves surviving input handles usable");
                assert!(matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ));
            }
        } else {
            assert_eq!(control_peer.read(&mut byte).unwrap(), 0);
            assert_eq!(data_peer.read(&mut byte).unwrap(), 0);
        }
    }
}
