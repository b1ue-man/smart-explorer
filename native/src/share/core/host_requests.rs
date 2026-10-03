//! Dispatcher entry points of RV1 host operations.
//! The dispatcher admits authenticated principals, export access and own stages.
use std::{io, sync::{Arc, atomic::AtomicBool}};
use iroh::endpoint::SendStream;
use super::{fs::ResolvedTarget, fs_access::FsAccess, session::PeerPrincipal,
    wire::{FsDuplicateSearch, FsHashWalk, FsListBatch, FsRecycle, FsResponse, FsStageFinish, FsSyncFilesystem, FsWatch}};

pub(in crate::share) async fn serve_duplicate_search(send: SendStream, request: FsDuplicateSearch, access: FsAccess, principal: PeerPrincipal) -> io::Result<()> {
    super::analysis_tasks::serve_duplicates(send,request,access,principal).await
}
pub(in crate::share) async fn serve_hash_walk(send: SendStream, request: FsHashWalk, access: FsAccess, principal: PeerPrincipal) -> io::Result<()> {
    let p = crate::analytics::Progress::default();
    let worker = p.clone();
    super::host_stream::serve(send,principal,access.clone(),p.cancel.clone(),false,
        || FsResponse::HashWalk { message: super::fs_response::FsHashWalkMessage::Progress {
            files:p.files.load(std::sync::atomic::Ordering::Relaxed),bytes:p.bytes.load(std::sync::atomic::Ordering::Relaxed) } },
        move |tx,cancel| super::host_hash_walk::run(request,access,tx,cancel,worker)).await
}
pub(in crate::share) async fn serve_list_batch(send: SendStream, request: FsListBatch, access: FsAccess, principal: PeerPrincipal) -> io::Result<()> {
    super::host_stream::serve(send,principal,access.clone(),Arc::new(AtomicBool::new(false)),true,
        || FsResponse::EntriesBatch { entries:Vec::new(),omitted:Vec::new() },
        move |tx,cancel| super::host_list::run(request,access,tx,&cancel)).await
}
pub(in crate::share) async fn serve_watch(send: SendStream, request: FsWatch, access: FsAccess, principal: PeerPrincipal) -> io::Result<()> {
    super::host_watch::serve(send,request,access,principal).await
}
pub(in crate::share) fn serve_recycle(target: ResolvedTarget, request: FsRecycle) -> io::Result<FsResponse> { super::host_mutations::recycle(target,request) }
pub(in crate::share) fn serve_finish_stage(target: ResolvedTarget, request: FsStageFinish) -> io::Result<FsResponse> { super::host_mutations::finish(target,request) }
pub(in crate::share) fn serve_sync_filesystem(target: ResolvedTarget, request: FsSyncFilesystem) -> io::Result<FsResponse> { super::host_mutations::sync(target,request) }
