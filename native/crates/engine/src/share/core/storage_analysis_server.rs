//! Authorized analysis dispatch; host scheduler owns execution and retention.
use super::{fs_access::FsAccess, session::PeerPrincipal, wire::FsStorageAnalysis};
use iroh::endpoint::SendStream;
use std::io;

pub(super) async fn serve(
    send: SendStream,
    request: FsStorageAnalysis,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    super::analysis_tasks::serve_analysis(send, request, access, principal).await
}

#[cfg(test)]
#[path = "storage_analysis_task_tests.rs"]
mod task_tests;

#[cfg(all(test, windows))]
#[path = "../os/windows/storage_analysis_task_tests.rs"]
mod windows_task_tests;
