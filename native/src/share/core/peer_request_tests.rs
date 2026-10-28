//! Which Share requests are replayed and which replies match them.
use super::*;

#[test]
fn remote_drive_task_only_idempotent_control_reads_are_replayable() {
    assert!(is_retryable_read(&FsRequest::ListDir { path: "/".into() }));
    assert!(is_retryable_read(&FsRequest::Stat { path: "/".into() }));
    assert!(is_retryable_read(&FsRequest::Capabilities {
        path: "/Docs".into(),
        acquire_lease: true,
        lease_request_id: Some("mount-request".into()),
    }));
    assert!(!is_retryable_read(&FsRequest::Read {
        path: "/x".into(),
        lz4: false,
    }));
    assert!(!is_retryable_read(&FsRequest::Write {
        path: "/x".into(),
        lz4: false,
    }));
    assert!(!is_retryable_read(&FsRequest::Rename {
        src: "/a".into(),
        dst: "/b".into(),
    }));
    assert!(!is_retryable_read(&FsRequest::RemoveFile {
        path: "/x".into()
    }));
    assert!(!is_retryable_read(&FsRequest::ReleaseLease));
}

#[test]
fn transfer_engine_task_only_batch_status_is_replayable() {
    assert!(is_retryable_read(&FsRequest::PutBatchStatus {
        nonce: "0123456789abcdef".into(),
    }));
    assert!(!is_retryable_read(&FsRequest::PutBatch {
        nonce: "0123456789abcdef".into(),
        entries: Vec::new(),
        evidence: false,
    }));
    assert!(!is_retryable_read(&FsRequest::CreateDir {
        path: "/A/x".into(),
        exclusive: true,
    }));
    assert!(!is_retryable_read(&FsRequest::PromoteNoReplace {
        staged: "/A/x.stage".into(),
        destination: "/A/x".into(),
        copy: true,
        evidence: false,
    }));
    assert!(response_matches(
        &FsRequest::PutBatchStatus {
            nonce: "0123456789abcdef".into(),
        },
        &FsResponse::Batch {
            status: super::super::wire::FsBatchStatus::Pending,
        }
    ));
    assert!(!response_matches(
        &FsRequest::CreateDir {
            path: "/A/x".into(),
            exclusive: false,
        },
        &FsResponse::Ready
    ));
    assert!(response_matches(
        &FsRequest::DiscardStage {
            path: "/A/x.stage".into(),
        },
        &FsResponse::Ok
    ));
}
