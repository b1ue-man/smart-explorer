//! Cross-export copying retains both resolved backends through acknowledged commit.
use std::io;

use iroh::endpoint::SendStream;

use super::fs_access::FsAccess;
use super::mount_lease::{run_authorized, MountLeaseAuthorization};
use super::wire::FsResponse;

pub(super) async fn serve(
    send: &mut SendStream,
    source: String,
    destination: String,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
) -> io::Result<()> {
    let result = super::blocking::run("Share copy file", move || {
        run_authorized(authorization.as_ref(), || access.copy_file(&source, &destination))
    }).await;
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
