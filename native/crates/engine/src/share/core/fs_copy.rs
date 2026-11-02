//! Cross-export copying retains both resolved backends through acknowledged commit.
use std::io;

use iroh::endpoint::SendStream;

use super::fs_access::FsAccess;
use super::mount_lease::{run_authorized, MountLeaseAuthorization};
use super::wire::FsResponse;

/// The whole copy runs on the host while `slot` (its transfer admission) is
/// held; the reply follows once the copy is complete.
pub(super) async fn serve<G: Send + 'static>(
    send: &mut SendStream,
    source: String,
    destination: String,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
    slot: G,
) -> io::Result<()> {
    let result = super::blocking::run_holding("Share copy file", slot, move || {
        run_authorized(authorization.as_ref(), || {
            access.copy_file(&source, &destination)
        })
    })
    .await;
    match result {
        Ok(size) => super::framing::reply(send, FsResponse::Data { size }).await,
        Err(error) => super::framing::reply_err(send, error).await,
    }
}

pub(super) fn copy_between(
    source: &dyn crate::vfs::Backend,
    source_path: &str,
    destination: &dyn crate::vfs::Backend,
    destination_path: &str,
) -> io::Result<u64> {
    crate::vfs::copy_between(source, source_path, destination, destination_path)
}
