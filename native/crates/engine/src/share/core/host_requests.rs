//! Dispatcher entry points of RV1 host operations.
//! The dispatcher admits authenticated principals, export access and own stages.
use super::{
    fs::ResolvedTarget,
    fs_access::FsAccess,
    session::PeerPrincipal,
    wire::{
        FsContentSearch, FsDuplicateSearch, FsHashWalk, FsListBatch, FsRecycle, FsRequest,
        FsResponse, FsStageFinish, FsSyncFilesystem, FsWatch,
    },
};
use iroh::endpoint::SendStream;
use std::{
    io,
    sync::{atomic::AtomicBool, Arc},
};

/// The long RV1 host streams (`DuplicateSearch`, `HashWalk`, `ListDirBatch`,
/// `WatchExport`, `ContentSearch`): each on its fair workers.
pub(in crate::share) async fn serve_stream(
    send: SendStream,
    request: FsRequest,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    match request {
        FsRequest::DuplicateSearch(request) => {
            serve_duplicate_search(send, request, access, principal).await
        }
        FsRequest::HashWalk(request) => serve_hash_walk(send, request, access, principal).await,
        FsRequest::ListDirBatch(request) => {
            serve_list_batch(send, request, access, principal).await
        }
        FsRequest::WatchExport(request) => serve_watch(send, request, access, principal).await,
        FsRequest::ContentSearch(request) => {
            serve_content_search(send, request, access, principal).await
        }
        _ => Err(super::core::eio("Host-Strom falsch zugeordnet")),
    }
}
pub(in crate::share) async fn serve_duplicate_search(
    send: SendStream,
    request: FsDuplicateSearch,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    super::analysis_tasks::serve_duplicates(send, request, access, principal).await
}
pub(in crate::share) async fn serve_hash_walk(
    send: SendStream,
    request: FsHashWalk,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let p = crate::analytics::Progress::default();
    let worker = p.clone();
    super::host_stream::serve(
        send,
        principal,
        access.clone(),
        p.cancel.clone(),
        false,
        || FsResponse::HashWalk {
            message: super::fs_response::FsHashWalkMessage::Progress {
                files: p.files.load(std::sync::atomic::Ordering::Relaxed),
                bytes: p.bytes.load(std::sync::atomic::Ordering::Relaxed),
            },
        },
        move |tx, cancel| super::host_hash_walk::run(request, access, tx, cancel, worker),
    )
    .await
}
/// `content_search_v1`: the host searches below the request path within the
/// export's rights; heartbeats carry its running totals.
pub(in crate::share) async fn serve_content_search(
    send: SendStream,
    request: FsContentSearch,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let totals = Arc::new(super::host_content_search::Totals::default());
    let worker = totals.clone();
    super::host_stream::serve(
        send,
        principal,
        access.clone(),
        Arc::new(AtomicBool::new(false)),
        false,
        move || totals.progress(),
        move |tx, cancel| super::host_content_search::run(request, access, tx, cancel, worker),
    )
    .await
}
pub(in crate::share) async fn serve_list_batch(
    send: SendStream,
    request: FsListBatch,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    super::host_stream::serve(
        send,
        principal,
        access.clone(),
        Arc::new(AtomicBool::new(false)),
        true,
        || FsResponse::EntriesBatch {
            entries: Vec::new(),
            omitted: Vec::new(),
        },
        move |tx, cancel| super::host_list::run(request, access, tx, &cancel),
    )
    .await
}
pub(in crate::share) async fn serve_watch(
    send: SendStream,
    request: FsWatch,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    super::host_watch::serve(send, request, access, principal).await
}
pub(in crate::share) fn serve_recycle(
    target: ResolvedTarget,
    request: FsRecycle,
) -> io::Result<FsResponse> {
    super::host_mutations::recycle(target, request)
}
pub(in crate::share) fn serve_finish_stage(
    target: ResolvedTarget,
    request: FsStageFinish,
) -> io::Result<FsResponse> {
    super::host_mutations::finish(target, request)
}
pub(in crate::share) fn serve_sync_filesystem(
    target: ResolvedTarget,
    request: FsSyncFilesystem,
) -> io::Result<FsResponse> {
    super::host_mutations::sync(target, request)
}
