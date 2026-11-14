//! Windows task tests of confined watches from a held directory pin (M14,
//! WIN-SHARE-WATCH): changes, root identity, link children and the
//! `Unsupported` mapping that lets a subscriber switch to polling.

use std::path::Path;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use super::*;
use crate::local_access::DirectoryHandle;
use crate::watch::EventKind;

const WAIT: Duration = Duration::from_secs(10);

fn start(pin: &DirectoryHandle, root: &Path) -> (WatchHandle, Receiver<WatchMessage>) {
    let (sender, receiver) = crossbeam_channel::bounded(1024);
    let handle = watch_confined(
        pin,
        root,
        WatchOptions::default(),
        WatchFilter::all(),
        WatchSink::from(sender),
    )
    .unwrap();
    (handle, receiver)
}

/// The first event of this watch that `wanted` accepts.
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

fn armed(receiver: &Receiver<WatchMessage>, handle: &WatchHandle) -> Option<WatchEvent> {
    wait_for(
        receiver,
        handle,
        |event| matches!(event, WatchEvent::Ready(_) | WatchEvent::Unavailable(_)),
        WAIT,
    )
}

fn change(rel: &'static str, kind: EventKind) -> impl Fn(&WatchEvent) -> bool {
    move |event| matches!(event, WatchEvent::Change(change) if change.rel == rel && change.kind == kind)
}

#[test]
fn windows_watch_task_confined_reports_create_modify_rename_delete() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let pin = DirectoryHandle::open_root(root).unwrap();
    let (handle, events) = start(&pin, root);
    // Before M14 this was `Unavailable(Unsupported)` on every Windows host.
    assert_eq!(
        armed(&events, &handle),
        Some(WatchEvent::Ready(Coverage::Complete))
    );

    std::fs::write(root.join("a.txt"), b"one").unwrap();
    assert!(wait_for(&events, &handle, change("a.txt", EventKind::Created), WAIT).is_some());
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(root.join("a.txt"))
            .unwrap();
        file.write_all(b" two").unwrap();
        file.sync_all().unwrap();
    }
    assert!(wait_for(&events, &handle, change("a.txt", EventKind::Modified), WAIT).is_some());
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("b.txt"), b"x").unwrap();
    // The whole subtree is watched, not only the immediate entries.
    assert!(wait_for(
        &events,
        &handle,
        change("sub/b.txt", EventKind::Created),
        WAIT
    )
    .is_some());
    std::fs::rename(root.join("a.txt"), root.join("c.txt")).unwrap();
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
        change("c.txt", EventKind::RenamedTo),
        WAIT
    )
    .is_some());
    std::fs::remove_file(root.join("c.txt")).unwrap();
    assert!(wait_for(&events, &handle, change("c.txt", EventKind::Removed), WAIT).is_some());

    // Dropping the watch cancels its read and closes its handle.
    let id = handle.id();
    drop(handle);
    std::thread::sleep(Duration::from_millis(200));
    while events.try_recv().is_ok() {}
    std::fs::write(root.join("after.txt"), b"x").unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match events.recv_timeout(left) {
            Ok(message) => assert_ne!(message.id, id, "event after drop: {message:?}"),
            Err(_) => break,
        }
    }
    drop(pin);
}

#[test]
fn windows_watch_task_replaced_root_is_refused() {
    let held = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let pin = DirectoryHandle::open_root(held.path()).unwrap();
    // A path that names another object than the pin never becomes the root.
    let refused = pin
        .open_watch_at(other.path())
        .err()
        .expect("another directory must be refused");
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
    // Any spelling of the pinned object itself is accepted.
    assert!(pin.open_watch_at(held.path()).is_ok());
    assert!(pin.open_watch().is_ok());
    // While held, the root cannot be renamed away and replaced.
    let (handle, events) = start(&pin, held.path());
    assert_eq!(
        armed(&events, &handle),
        Some(WatchEvent::Ready(Coverage::Complete))
    );
    let moved = held.path().with_extension("moved");
    assert!(std::fs::rename(held.path(), &moved).is_err());
    std::fs::write(held.path().join("still.txt"), b"x").unwrap();
    assert!(wait_for(
        &events,
        &handle,
        change("still.txt", EventKind::Created),
        WAIT
    )
    .is_some());
}

#[test]
fn windows_watch_task_link_child_is_not_followed() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("plain")).unwrap();
    // A directory junction needs no privilege (unlike a symbolic link).
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(root.join("link"))
        .arg(outside.path())
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J failed: {status}");
    let pin = DirectoryHandle::open_root(root).unwrap();
    assert!(pin.under_link("link/inner.txt"));
    assert!(pin.under_link("link/deeper/inner.txt"));
    assert!(!pin.under_link("link"));
    assert!(!pin.under_link("plain/inner.txt"));
    assert!(!pin.under_link("top.txt"));

    let (handle, events) = start(&pin, root);
    assert_eq!(
        armed(&events, &handle),
        Some(WatchEvent::Ready(Coverage::Complete))
    );
    std::fs::write(outside.path().join("via-target.txt"), b"x").unwrap();
    std::fs::write(root.join("link").join("via-link.txt"), b"x").unwrap();
    std::fs::write(root.join("plain").join("marker.txt"), b"x").unwrap();
    let mut seen = Vec::new();
    let deadline = Instant::now() + WAIT;
    let mut marker_at = None;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if marker_at.is_some_and(|at: Instant| at.elapsed() > Duration::from_millis(500)) {
            break;
        }
        match events.recv_timeout(left.min(Duration::from_millis(100))) {
            Ok(message) if message.id == handle.id() => {
                if let WatchEvent::Change(change) = message.event {
                    if change.rel == "plain/marker.txt" && marker_at.is_none() {
                        marker_at = Some(Instant::now());
                    }
                    seen.push(change.rel);
                }
            }
            Ok(_) | Err(_) => {}
        }
    }
    assert!(marker_at.is_some(), "marker change missing: {seen:?}");
    assert!(
        !seen.iter().any(|rel| rel.starts_with("link/")),
        "a change below the junction was reported as root content: {seen:?}"
    );
}

#[test]
fn windows_watch_task_missing_notifications_end_with_unsupported() {
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED,
    };
    // Microsoft: a redirector or file system without the operation fails
    // with ERROR_INVALID_FUNCTION; the Share host passes `Unsupported` on so
    // that the subscriber polls instead of subscribing again.
    for code in [
        ERROR_INVALID_FUNCTION,
        ERROR_NOT_SUPPORTED,
        ERROR_INVALID_PARAMETER,
    ] {
        assert_eq!(
            handle_watch::reason_of_code(code),
            UnavailableReason::Unsupported
        );
    }
    assert_eq!(
        handle_watch::reason_of_code(ERROR_ACCESS_DENIED),
        UnavailableReason::AccessDenied
    );
}
