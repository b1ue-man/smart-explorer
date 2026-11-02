//! DSK-M2: one update-stop rule for `se mcp`, every GUI window and every
//! background process, while the updater helper itself stays unchanged.
use super::{is_new_request, UpdateStopBaseline, UpdateStopSource};
use sha2::{Digest, Sha256};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type FakeMarker = Arc<Mutex<Option<SystemTime>>>;

fn fake_marker(initial: Option<SystemTime>) -> (FakeMarker, UpdateStopSource) {
    let marker = Arc::new(Mutex::new(initial));
    let reader = marker.clone();
    let source: UpdateStopSource = Arc::new(move || *reader.lock().unwrap());
    (marker, source)
}

fn set(marker: &FakeMarker, value: Option<SystemTime>) {
    *marker.lock().unwrap() = value;
}

#[test]
fn desktop_task_update_stop_rule_matches_mcp() {
    let old = Some(UNIX_EPOCH + Duration::from_secs(10));
    let new = Some(UNIX_EPOCH + Duration::from_secs(20));
    // Every pair of marker states at start and later. The rows of
    // `mcp_task_only_new_update_stop_requests_end_the_server` are a subset.
    let table = [
        (None, None, false),
        (None, old, true),
        (None, new, true),
        (old, None, false),
        (old, old, false),
        (old, new, true),
        (new, None, false),
        (new, old, true),
        (new, new, false),
    ];
    for (baseline, current, expected) in table {
        assert_eq!(
            is_new_request(baseline, current),
            expected,
            "{baseline:?} -> {current:?}"
        );
        // The se shell (`cli/shell_repl.rs`, `cli/shell_terminal.rs`) states
        // the same rule as `current.is_some() && current != baseline`.
        assert_eq!(current.is_some() && current != baseline, expected);
        // The baseline object applies the same rule to its marker source.
        let (marker, source) = fake_marker(baseline);
        let armed = UpdateStopBaseline::with_source(source);
        set(&marker, current);
        assert_eq!(armed.requested(), expected, "{baseline:?} -> {current:?}");
    }
}

#[test]
fn update_stop_watch_fires_once_and_ends_with_its_handle() {
    let stale = Some(UNIX_EPOCH + Duration::from_secs(5));
    let (marker, source) = fake_marker(stale);
    let (sender, received) = mpsc::channel();
    let watch = UpdateStopBaseline::with_source(source)
        .spawn_watch(Duration::from_millis(10), move || {
            let _ = sender.send(());
        })
        .unwrap();
    // The marker present at start is not a request.
    assert_eq!(
        received.recv_timeout(Duration::from_millis(150)),
        Err(RecvTimeoutError::Timeout)
    );
    assert!(!watch.fired());
    set(&marker, Some(UNIX_EPOCH + Duration::from_secs(6)));
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(watch.fired());
    // The callback ran once and the thread ended with it.
    assert_eq!(
        received.recv_timeout(Duration::from_secs(5)),
        Err(RecvTimeoutError::Disconnected)
    );
    watch.stop();

    // Dropping the handle ends the thread long before its first read.
    let (_marker, source) = fake_marker(None);
    let (sender, received) = mpsc::channel::<()>();
    let watch = UpdateStopBaseline::with_source(source)
        .spawn_watch(Duration::from_secs(3600), move || {
            let _ = sender.send(());
        })
        .unwrap();
    drop(watch);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(5)),
        Err(RecvTimeoutError::Disconnected)
    );
}
