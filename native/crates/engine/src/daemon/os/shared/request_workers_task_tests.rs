use super::request_workers::RequestWorkers;
use std::io;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[test]
fn remote_drive_task_worker_shutdown_waits_for_running_writer() {
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
