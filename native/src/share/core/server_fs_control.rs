//! Existing authorized control operations and response helpers.
use std::io;
use iroh::endpoint::SendStream;
use crate::share::{
    framing::{reply, reply_err},
    fs::ResolvedTarget,
    fs_access::FsAccess,
    mount_lease::{run_authorized, MountLeaseAuthorization},
    session::PeerPrincipal,
    wire::FsResponse,
};

pub(super) async fn simple<F>(
    send: &mut SendStream,
    path: String,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
    principal: &PeerPrincipal,
    label: &'static str,
    operation: F,
) -> io::Result<()>
where
    F: FnOnce(ResolvedTarget) -> io::Result<()> + Send + 'static,
{
    let result = control(principal, label, move || {
        run_authorized(authorization.as_ref(), || {
            access.resolve_write(&path).and_then(operation)
        })
    })
    .await;
    reply_unit(send, result).await
}

/// Like `simple`, for an operation that builds its own reply.
pub(super) async fn answer<F>(
    send: &mut SendStream,
    path: String,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
    principal: &PeerPrincipal,
    label: &'static str,
    operation: F,
) -> io::Result<()>
where
    F: FnOnce(ResolvedTarget) -> io::Result<FsResponse> + Send + 'static,
{
    let result = control(principal, label, move || {
        run_authorized(authorization.as_ref(), || {
            access.resolve_write(&path).and_then(operation)
        })
    })
    .await;
    match result {
        Ok(response) => reply(send, response).await,
        Err(error) => reply_err(send, error).await,
    }
}

pub(super) async fn reply_unit(send: &mut SendStream, result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => reply(send, FsResponse::Ok).await,
        Err(error) => reply_err(send, error).await,
    }
}

pub(super) async fn control<T, F>(principal: &PeerPrincipal, label: &'static str, operation: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    let class = if matches!(label, "Share remove directory" | "Share recycle" | "Share sync filesystem") {
        crate::share::blocking::Class::Background
    } else { crate::share::blocking::Class::Control };
    crate::share::blocking::run_for(principal.clone(), class, label, operation).await
}
