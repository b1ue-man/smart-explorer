use super::{FsRequest, FsResponse};

pub(super) fn is_retryable_read(request: &FsRequest) -> bool {
    matches!(
        request,
        FsRequest::Capabilities { .. }
            | FsRequest::ListDir { .. }
            | FsRequest::DirectoryIdentity { .. }
            | FsRequest::FileIdentity { .. }
            | FsRequest::DirectoryReservationSupported { .. }
            | FsRequest::Capacity { .. }
            | FsRequest::Stat { .. }
            | FsRequest::SyncChildPath { .. }
            | FsRequest::PutBatchStatus { .. }
    )
}

#[cfg(test)]
mod reservation_compatibility_tests {
    use super::*;
    #[test]
    fn batches_task_transfer_compat_reserved_share_rename_accepts_only_unit_reply_and_never_read_retries() {
        let request = FsRequest::RenameReservedDirectory { src: "/owned".into(), dst: "/final".into() };
        assert!(!is_retryable_read(&request));
        assert!(response_matches(&request, &FsResponse::Ok));
        assert!(!response_matches(&request, &FsResponse::DirectoryReservationSupported { supported: true }));
    }
}

pub(super) fn response_matches(request: &FsRequest, response: &FsResponse) -> bool {
    if matches!(response, FsResponse::Err { .. }) {
        return true;
    }
    match request {
        FsRequest::DirectoryReservationSupported { .. } => matches!(response, FsResponse::DirectoryReservationSupported { .. }),
        FsRequest::DirectoryIdentity { .. } => matches!(response, FsResponse::DirectoryIdentity { .. }),
        FsRequest::FileIdentity { .. } => matches!(response, FsResponse::FileIdentity { .. }),
        FsRequest::Capacity { .. } => matches!(response, FsResponse::Capacity { .. }),
        FsRequest::Capabilities { .. } => matches!(response, FsResponse::Capabilities { .. }),
        FsRequest::ListDir { .. } => matches!(response, FsResponse::Entries { .. }),
        FsRequest::Stat { .. } => matches!(response, FsResponse::Meta { .. }),
        FsRequest::SyncChildPath { .. } => matches!(response, FsResponse::ChildPath { .. }),
        FsRequest::ReplaceStagedReversible(_) => {
            matches!(response, FsResponse::ReversibleReplaced { .. })
        }
        FsRequest::CopyFile { .. } => {
            matches!(response, FsResponse::Data { .. } | FsResponse::Ok)
        }
        FsRequest::Rename { .. }
        | FsRequest::RenameNoReplace { .. }
        | FsRequest::RenameReservedDirectory { .. }
        | FsRequest::PromoteStaged { .. }
        | FsRequest::PromoteNoReplace { .. }
        | FsRequest::RemoveFile { .. }
        | FsRequest::RemoveDir { .. }
        | FsRequest::MkdirAll { .. }
        | FsRequest::CreateDir { .. }
        | FsRequest::CreateDirExclusive { .. }
        | FsRequest::DiscardStage { .. }
        | FsRequest::ReleaseLease => matches!(response, FsResponse::Ok),
        FsRequest::PutBatchStatus { .. } => matches!(response, FsResponse::Batch { .. }),
        FsRequest::SegmentedStageOpen(_) => {
            matches!(response, FsResponse::SegmentedStageReady { .. })
        }
        FsRequest::SegmentedStageFinish(_) => matches!(response, FsResponse::Ok),
        FsRequest::Recycle(_) => matches!(response, FsResponse::Recycle { .. }),
        FsRequest::FinishStage(_) => matches!(response, FsResponse::StageFinished { .. }),
        FsRequest::SyncFilesystem(_) => matches!(response, FsResponse::Synced { .. }),
        FsRequest::Read { .. }
        | FsRequest::ReadAt { .. }
        | FsRequest::Write { .. }
        | FsRequest::WriteNew { .. }
        | FsRequest::WriteDone
        | FsRequest::StageWriteAt(_)
        | FsRequest::WalkTree { .. }
        | FsRequest::StorageSnapshot { .. }
        | FsRequest::StorageAnalysis(_)
        | FsRequest::DuplicateSearch(_)
        | FsRequest::HashWalk(_)
        | FsRequest::ListDirBatch(_)
        | FsRequest::WatchExport(_)
        | FsRequest::RemoveTree { .. }
        | FsRequest::PutBatch { .. }
        | FsRequest::GetBatch { .. } => false,
    }
}
