use super::wire::{FsRequest, FsResponse};

pub(super) fn request_label(request: &FsRequest) -> &'static str {
    match request {
        FsRequest::Capabilities { .. } => "capabilities",
        FsRequest::ReleaseLease => "release_lease",
        FsRequest::ListDir { .. } => "list_dir",
        FsRequest::Stat { .. } => "stat",
        FsRequest::WalkTree { .. } => "walk_tree",
        FsRequest::StorageSnapshot { .. } => "storage_snapshot",
        FsRequest::StorageAnalysis(_) => "storage_analysis",
        FsRequest::Read { .. } => "read",
        FsRequest::Write { .. } => "write",
        FsRequest::WriteNew { .. } => "write_new",
        FsRequest::WriteDone => "write_done",
        FsRequest::MkdirAll { .. } => "mkdir_all",
        FsRequest::Rename { .. } => "rename",
        FsRequest::RenameNoReplace { .. } => "rename_no_replace",
        FsRequest::PromoteStaged { .. } => "promote_staged",
        FsRequest::CopyFile { .. } => "copy_file",
        FsRequest::RemoveFile { .. } => "remove_file",
        FsRequest::RemoveDir { .. } => "remove_dir",
        FsRequest::PutBatch { .. } => "put_batch",
        FsRequest::PutBatchStatus { .. } => "put_batch_status",
        FsRequest::GetBatch { .. } => "get_batch",
        FsRequest::ReadAt { .. } => "read_at",
        FsRequest::CreateDir { .. } => "create_dir",
        FsRequest::PromoteNoReplace { .. } => "promote_no_replace",
        FsRequest::DiscardStage { .. } => "discard_stage",
        FsRequest::DuplicateSearch(_) => "duplicate_search",
        FsRequest::HashWalk(_) => "hash_walk",
        FsRequest::ListDirBatch(_) => "list_dir_batch",
        FsRequest::Recycle(_) => "recycle",
        FsRequest::FinishStage(_) => "finish_stage",
        FsRequest::SyncFilesystem(_) => "sync_filesystem",
        FsRequest::WatchExport(_) => "watch_export",
    }
}

pub(super) fn response_summary(response: &FsResponse) -> String {
    match response {
        FsResponse::Capabilities {
            capabilities,
            contract_version,
            root_confined,
            lease,
            storage_snapshot_v1,
            ..
        } => format!(
            concat!(
                "capabilities contract={} root_confined={} lease={} ",
                "storage_snapshot_v1={} create={} replace={} namespace_replace={} ",
                "access={:?} features={}"
            ),
            contract_version,
            root_confined,
            lease.is_some(),
            storage_snapshot_v1,
            capabilities.create,
            capabilities.replace,
            capabilities.namespace_replace,
            capabilities.access,
            capabilities.features.names().join(","),
        ),
        FsResponse::Entries { entries } => format!("{} Eintraege", entries.len()),
        FsResponse::Meta { meta } => format!("meta size={} dir={}", meta.size, meta.is_dir),
        FsResponse::WalkBatch {
            nodes,
            files,
            dirs,
            bytes,
        } => format!(
            "walk nodes={} files={files} dirs={dirs} bytes={bytes}",
            nodes.len()
        ),
        FsResponse::WalkDone {
            files,
            dirs,
            bytes,
            nodes,
        } => format!("walk done nodes={nodes} files={files} dirs={dirs} bytes={bytes}"),
        FsResponse::SnapshotProgress {
            files,
            dirs,
            bytes,
            nodes,
        } => {
            format!("snapshot progress nodes={nodes} files={files} dirs={dirs} bytes={bytes}")
        }
        FsResponse::SnapshotReady {
            encoded_len,
            files,
            dirs,
            bytes,
            nodes,
            ..
        } => {
            format!(
                concat!(
                    "snapshot ready encoded={} nodes={} ",
                    "files={} dirs={} bytes={}"
                ),
                encoded_len, nodes, files, dirs, bytes,
            )
        }
        FsResponse::SnapshotDone {
            files,
            dirs,
            bytes,
            nodes,
        } => {
            format!("snapshot done nodes={nodes} files={files} dirs={dirs} bytes={bytes}")
        }
        FsResponse::Analysis { .. } => "storage analysis".into(),
        FsResponse::Data { size } => format!("{size} bytes"),
        FsResponse::Ready => "bereit".into(),
        FsResponse::Ok => "ok".into(),
        FsResponse::Batch { status } => status.summary(),
        FsResponse::Recycle { moved } => format!("papierkorb moved={moved}"),
        FsResponse::StageFinished {
            mtime_applied,
            durable,
        } => format!("stufe fertig mtime={mtime_applied} durable={durable}"),
        FsResponse::Synced { durable } => format!("gesichert durable={durable}"),
        FsResponse::Err { msg, .. } => format!("fehler={msg}"),
    }
}
