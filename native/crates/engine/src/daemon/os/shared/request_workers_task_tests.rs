use super::request_workers::RequestWorkers;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[test]
fn remote_drive_task_worker_shutdown_waits_for_running_writer() {
    let cases: [fn(); 2] = [
        remote_drive_task_worker_shutdown_waits_for_running_writer_case,
        shutdown_joins_completed_cleanup,
    ];
    for case in cases {
        case();
    }
}

fn remote_drive_task_worker_shutdown_waits_for_running_writer_case() {
    let committed = Arc::new(Mutex::new(Vec::new()));
    let worker_committed = committed.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    let mut workers = RequestWorkers::default();
    workers.push(std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        worker_committed.lock().unwrap().extend_from_slice(b"saved");
        Ok(())
    }));
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let shutdown = std::thread::spawn(move || {
        let result: io::Result<()> = workers.shutdown();
        shutdown_tx.send(result).unwrap();
    });
    assert!(shutdown_rx.recv_timeout(Duration::from_millis(25)).is_err());

    release_tx.send(()).unwrap();
    shutdown_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    shutdown.join().unwrap();
    assert_eq!(committed.lock().unwrap().as_slice(), b"saved");
}

fn shutdown_joins_completed_cleanup() {
    let completed = Arc::new(AtomicBool::new(false));
    let worker_completed = completed.clone();
    let mut workers = RequestWorkers::default();
    workers.push(std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        worker_completed.store(true, Ordering::Relaxed);
        Ok(())
    }));
    workers.shutdown().unwrap();
    assert!(completed.load(Ordering::Relaxed));
}
