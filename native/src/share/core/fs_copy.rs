//! Cross-export copying retains both resolved backends through acknowledged commit.
use std::io::{self, Write};

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
    let before = source.stat(source_path)?;
    if before.is_dir || before.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Share copy source must be a regular file",
        ));
    }
    let mut input = source.open_read_id(source_path, before.id.as_deref())?;
    let staging = crate::vfs::unique_staging_path(destination, destination_path, "copy")?;
    // An absence probe does not confer ownership. Only exclusive creation may
    // open this spelling, and later errors never authorize deleting it by path.
    let mut output = destination.open_write_new(&staging)?;
    let copied = io::copy(&mut input, &mut output)?;
    let after = source.stat(source_path)?;
    if copied != before.size || after.is_dir || after.is_symlink
        || after.size != before.size || after.mtime_ms != before.mtime_ms
        || after.id != before.id || after.content_md5 != before.content_md5
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Share copy source changed during transfer",
        ));
    }
    // Peer/provider writers may commit only on flush. Its error must reach the
    // caller; drop alone must never be mistaken for an acknowledged upload.
    output.flush()?;
    drop(output);
    drop(input);
    crate::vfs::promote_staged_replace(destination, &staging, destination_path)?;
    Ok(copied)
}
