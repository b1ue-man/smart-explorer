//! Milestone tests of the inotify watch service on Linux (RV1, T-JOBS).

use std::path::Path;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use super::*;

const WAIT: Duration = Duration::from_secs(10);

fn start(
    root: &Path,
    filter: WatchFilter,
    capacity: usize,
) -> (WatchHandle, Receiver<WatchMessage>) {
    let (sender, receiver) = crossbeam_channel::bounded(capacity);
    let handle = watch(
        root,
        WatchOptions::default(),
        filter,
        WatchSink::from(sender),
    )
    .unwrap();
    (handle, receiver)
}

/// Waits for the first event of this watch that `wanted` accepts.
fn wait_for(
    receiver: &Receiver<WatchMessage>,
    handle: &WatchHandle,
    wanted: impl Fn(&WatchEvent) -> bool,
    timeout: Duration,
) -> Option<WatchEvent> {
    let deadline = Instant::now() + timeout;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok(message) if message.id == handle.id() && wanted(&message.event) => {
                return Some(message.event)
            }
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

fn ready(receiver: &Receiver<WatchMessage>, handle: &WatchHandle) {
    let event = wait_for(
        receiver,
        handle,
        |event| matches!(event, WatchEvent::Ready(_)),
        WAIT,
    );
    assert!(
        matches!(event, Some(WatchEvent::Ready(_))),
        "watch armed: {event:?}"
    );
}

fn change(rel: &'static str, kind: EventKind) -> impl Fn(&WatchEvent) -> bool {
    move |event| matches!(event, WatchEvent::Change(change) if change.rel == rel && change.kind == kind)
}

#[test]
fn review_task_watch_reports_renames_folders_and_same_size_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("a.txt"), b"1234").unwrap();
    let (handle, events) = start(root, WatchFilter::all(), 1024);
    ready(&events, &handle);

    // A same-size edit keeps size and may keep the second of the mtime.
    std::fs::write(root.join("a.txt"), b"abcd").unwrap();
    assert!(wait_for(&events, &handle, change("a.txt", EventKind::Modified), WAIT).is_some());

    std::fs::rename(root.join("a.txt"), root.join("b.txt")).unwrap();
    assert!(wait_for(
        &events,
        &handle,
        change("a.txt", EventKind::RenamedFrom),
        WAIT
    )
    .is_some());
    assert!(wait_for(
        &events,
        &handle,
        change("b.txt", EventKind::RenamedTo),
        WAIT
    )
    .is_some());

    // A new folder is watched before it is read: its files are reported.
    std::fs::create_dir(root.join("neu")).unwrap();
    assert!(wait_for(&events, &handle, change("neu", EventKind::Created), WAIT).is_some());
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(root.join("neu/c.txt"), b"x").unwrap();
    assert!(wait_for(
        &events,
        &handle,
        change("neu/c.txt", EventKind::Created),
        WAIT
    )
    .is_some());

    // An empty folder alone is a change as well.
    std::fs::create_dir(root.join("leer")).unwrap();
    assert!(wait_for(&events, &handle, change("leer", EventKind::Created), WAIT).is_some());
}

#[test]
fn review_task_watch_filter_and_links_exclude_subtrees() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("cache")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
    let filter = WatchFilter::new(|entry| entry.rel != "cache" && !entry.rel.starts_with("cache/"));
    let (handle, events) = start(root, filter, 1024);
    ready(&events, &handle);

    std::fs::write(root.join("cache/temp.bin"), b"x").unwrap();
    std::fs::write(outside.path().join("fremd.txt"), b"x").unwrap();
    std::fs::write(root.join("seen.txt"), b"x").unwrap();

    let first = wait_for(
        &events,
        &handle,
        |event| matches!(event, WatchEvent::Change(_)),
        WAIT,
    );
    match first {
        Some(WatchEvent::Change(change)) => assert_eq!(change.rel, "seen.txt"),
        other => panic!("expected the change outside the excluded parts: {other:?}"),
    }
}

#[test]
fn review_task_watch_root_removal_is_reported_and_rearmed() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let (handle, events) = start(&root, WatchFilter::all(), 1024);
    ready(&events, &handle);

    std::fs::remove_dir(&root).unwrap();
    let missing = wait_for(
        &events,
        &handle,
        |event| matches!(event, WatchEvent::Unavailable(_)),
        WAIT,
    );
    assert_eq!(
        missing,
        Some(WatchEvent::Unavailable(UnavailableReason::RootMissing))
    );

    std::fs::create_dir(&root).unwrap();
    let again = wait_for(
        &events,
        &handle,
        |event| matches!(event, WatchEvent::Ready(_)),
        Duration::from_secs(30),
    );
    assert!(
        matches!(again, Some(WatchEvent::Ready(_))),
        "re-armed: {again:?}"
    );
}

#[test]
fn review_task_watch_full_channel_turns_into_one_overflow() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let (handle, events) = start(root, WatchFilter::all(), 4);
    ready(&events, &handle);
    for index in 0..64 {
        std::fs::write(root.join(format!("f{index}.txt")), b"x").unwrap();
    }
    std::thread::sleep(Duration::from_millis(500));
    let overflow = wait_for(
        &events,
        &handle,
        |event| matches!(event, WatchEvent::Overflow),
        WAIT,
    );
    assert_eq!(overflow, Some(WatchEvent::Overflow));
}

#[test]
fn review_task_watch_host_signal_reaches_the_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let (handle, events) = start(&root, WatchFilter::all(), 1024);
    ready(&events, &handle);
    report_host_change(&[root.join("DCIM/neu.jpg")]);
    assert!(wait_for(
        &events,
        &handle,
        change("DCIM/neu.jpg", EventKind::Unknown),
        WAIT
    )
    .is_some());

    set_host_cursor(&root, Some("v1:42".into()));
    assert_eq!(host_cursor(&root.join("DCIM")).as_deref(), Some("v1:42"));
    set_host_cursor(&root, None);
    assert_eq!(host_cursor(&root.join("DCIM")), None);
    drop(handle);
}

#[test]
fn review_task_watch_confined_never_reopens_a_swapped_root_or_ancestor() {
    for swap_ancestor in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let export = directory.path().join("export");
        let literal_root = export.join("child");
        std::fs::create_dir_all(literal_root.join("nested")).unwrap();
        let export_pin = crate::local_access::DirectoryHandle::open_root(&export).unwrap();
        let pin = export_pin.open_child(std::ffi::OsStr::new("child")).unwrap();
        let retained = directory.path().join("retained");
        let (retained_root, outside_root) = if swap_ancestor {
            std::fs::create_dir(outside.path().join("child")).unwrap();
            std::fs::rename(&export, &retained).unwrap();
            std::os::unix::fs::symlink(outside.path(), &export).unwrap();
            (retained.join("child"), outside.path().join("child"))
        } else {
            std::fs::rename(&literal_root, &retained).unwrap();
            std::os::unix::fs::symlink(outside.path(), &literal_root).unwrap();
            (retained, outside.path().to_path_buf())
        };
        let (sender, events) = crossbeam_channel::bounded(1024);
        let handle = watch_confined(&pin, &literal_root, WatchOptions::default(),
            WatchFilter::all(), WatchSink::from(sender)).unwrap();
        drop(pin);
        drop(export_pin);
        assert_eq!(wait_for(&events, &handle,
            |event| matches!(event, WatchEvent::Ready(_)), WAIT),
            Some(WatchEvent::Ready(Coverage::LocalOnly)));

        std::fs::write(outside_root.join("outside-name.txt"), b"x").unwrap();
        // A path-only signal cannot prove which directory object it names.
        report_host_change(&[literal_root.join("untrusted-host-name.txt")]);
        // Partial coverage deliberately avoids a path-based descendant walk.
        std::fs::write(retained_root.join("nested/deep.txt"), b"x").unwrap();
        std::fs::write(retained_root.join("authorized.txt"), b"x").unwrap();
        let first = wait_for(&events, &handle,
            |event| matches!(event, WatchEvent::Change(_)), WAIT);
        match first {
            Some(WatchEvent::Change(change)) => assert_eq!(change.rel, "authorized.txt"),
            other => panic!("expected an event from the held object: {other:?}"),
        }
    }
}

#[test]
fn review_task_watch_confined_overlap_preserves_recursive_job_watches() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let (job_watch, job_events) = start(root, WatchFilter::all(), 1024);
    ready(&job_events, &job_watch);
    let pin = crate::local_access::DirectoryHandle::open_root(root).unwrap();
    let (sender, events) = crossbeam_channel::bounded(1024);
    let confined = watch_confined(&pin, root, WatchOptions::default(),
        WatchFilter::all(), WatchSink::from(sender)).unwrap();
    ready(&events, &confined);
    drop(confined);
    drop(pin);
    std::fs::create_dir(root.join("new")).unwrap();
    assert!(wait_for(&job_events, &job_watch, change("new", EventKind::Created), WAIT).is_some());
    std::fs::write(root.join("new/deep.txt"), b"x").unwrap();
    assert!(wait_for(&job_events, &job_watch,
        change("new/deep.txt", EventKind::Created), WAIT).is_some());
}

#[test]
#[ignore = "Suite-Stufe: needs a container with a lowered fs.inotify.max_user_watches"]
fn review_task_watch_limit_falls_back_to_polling() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for index in 0..64 {
        std::fs::create_dir_all(root.join(format!("d{index}/e"))).unwrap();
    }
    let (handle, events) = start(root, WatchFilter::all(), 1024);
    let event = wait_for(
        &events,
        &handle,
        |event| matches!(event, WatchEvent::Ready(_) | WatchEvent::Unavailable(_)),
        WAIT,
    );
    assert_eq!(
        event,
        Some(WatchEvent::Unavailable(UnavailableReason::WatchLimit))
    );
}
