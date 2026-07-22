use super::{
    normalize_stderr, CapturedStderr, MOUNT_HOST_DETAIL_LIMIT, MOUNT_HOST_STDERR_LIMIT,
    TRUNCATED_MARKER,
};

#[test]
fn remote_drive_task_mount_host_stderr_keeps_a_bounded_actionable_tail() {
    let mut captured = CapturedStderr::default();
    captured.push_tail(&vec![b'x'; MOUNT_HOST_STDERR_LIMIT + 128]);
    captured.push_tail(
        b"\r\nsmart-explorer: internal mount host failed:\tRoot-Lease wurde vom Peer abgelehnt\r\n",
    );

    assert!(captured.truncated);
    assert_eq!(captured.bytes.len(), MOUNT_HOST_STDERR_LIMIT);
    assert_eq!(
        normalize_stderr(&captured.bytes, captured.truncated).as_deref(),
        Some("Root-Lease wurde vom Peer abgelehnt")
    );

    let mut unstructured = CapturedStderr::default();
    unstructured.push_tail(&vec![b'y'; MOUNT_HOST_STDERR_LIMIT + 1]);
    let detail = normalize_stderr(&unstructured.bytes, unstructured.truncated).unwrap();
    assert!(detail.starts_with(TRUNCATED_MARKER));
    assert!(detail.len() <= MOUNT_HOST_DETAIL_LIMIT);
}
