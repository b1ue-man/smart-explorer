use super::wire::{FsRequest, FsResponse};

pub(super) fn request_label(request: &FsRequest) -> &'static str {
    match request {
        FsRequest::Capabilities { .. } => "capabilities",
        FsRequest::ReleaseLease => "release_lease",
        FsRequest::ListDir { .. } => "list_dir",
        FsRequest::Stat { .. } => "stat",
        FsRequest::WalkTree { .. } => "walk_tree",
        FsRequest::StorageSnapshot { .. } => "storage_snapshot",
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
        } => format!(
            concat!(
                "capabilities contract={} root_confined={} lease={} ",
                "storage_snapshot_v1={} create={} replace={} namespace_replace={}"
            ),
            contract_version,
            root_confined,
            lease.is_some(),
            storage_snapshot_v1,
            capabilities.create,
            capabilities.replace,
            capabilities.namespace_replace,
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
        FsResponse::Data { size } => format!("{size} bytes"),
        FsResponse::Ready => "bereit".into(),
        FsResponse::Ok => "ok".into(),
        FsResponse::Err { msg, .. } => format!("fehler={msg}"),
    }
}
