//! The direct-open download of an edit, retried through the real daemon IPC. Moved
//! here from transfer/os/shared/edit_download_task_tests.rs: the daemon's bridge
//! lies above the transfer layer (plan docs/plaene/2026-11-01-idealzustand, D13).
use crate::transfer::download_for_edit;
use crate::transfer::edit_download_task_source::{assert_no_parts, Source};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[test]
fn direct_open_task_stream_retry_restarts_bytes_through_real_daemon_ipc() {
    let source = Arc::new(Source {
        failures: 1,
        ..Default::default()
    });
    let bridge = crate::daemon::DirectOpenTaskBridge::new(source.clone()).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("photo Ü #.jpg");
    let (_, revision) =
        download_for_edit(&*bridge.backend, "/A/photo Ü #.jpg", None, &destination).unwrap();
    assert_eq!(revision, 7);
    assert_eq!(source.opens.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&destination).unwrap(), source.bytes);
    assert_no_parts(temporary.path(), true);
}
