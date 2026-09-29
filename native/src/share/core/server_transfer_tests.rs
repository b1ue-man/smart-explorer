use super::*;

fn export(root: &std::path::Path) -> FsAccess {
    FsAccess::dynamic(crate::share::fs::ShareExportConfig {
        roots: vec![crate::share::fs::SharedRoot {
            label: "A".into(),
            path: root.to_string_lossy().replace('\\', "/"),
        }],
        include_connections: false,
    })
}

fn create_job(root: &std::path::Path, abandoned: bool) -> WriteJob {
    WriteJob {
        target: WriteTarget {
            path: "/A/late.bin".into(),
            mode: WriteMode::Create,
            authorization: None,
        },
        access: export(root),
        abandoned: Arc::new(AtomicBool::new(abandoned)),
    }
}

#[test]
fn transfer_engine_task_abandoned_write_creates_nothing() {
    let root = tempfile::tempdir().unwrap();

    // The client left while the host waited: the target never appears.
    let (ready_tx, ready_rx) = oneshot::channel();
    let (_commands, command_rx) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let (done_tx, _done_rx) = oneshot::channel();
    write_worker(create_job(root.path(), true), ready_tx, command_rx, done_tx).unwrap();
    let answer = ready_rx.blocking_recv().expect("the worker answers");
    assert_eq!(answer.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
    assert!(!root.path().join("late.bin").exists());

    // A client that still waits gets its target as before; an upload that
    // then breaks off keeps the exclusively created file (no identity to
    // prove it is still ours).
    let (ready_tx, ready_rx) = oneshot::channel();
    let (commands, command_rx) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let (done_tx, done_rx) = oneshot::channel();
    drop(commands);
    write_worker(
        create_job(root.path(), false),
        ready_tx,
        command_rx,
        done_tx,
    )
    .unwrap();
    assert!(ready_rx
        .blocking_recv()
        .expect("the worker answers")
        .is_ok());
    let done = done_rx.blocking_recv().expect("the worker ends");
    assert_eq!(done.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert!(root.path().join("late.bin").exists());
}
