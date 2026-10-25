//! Negotiated P14 operations. Every mutation is sent once; a missing reply
//! stays ambiguous and can only be recovered by comparing object identity.
use crate::share::{
    backend::PeerBackend, fs::ResolvedTarget, fs_access::FsAccess, peer_stream,
    wire::{FsRequest, FsResponse},
};
use std::io;

pub(super) fn supported(backend: &PeerBackend, root: &str) -> bool {
    if require_feature(backend, root).is_err() { return false; }
    matches!(backend.request(FsRequest::DirectoryReservationSupported { path: root.into() }),
        Ok(FsResponse::DirectoryReservationSupported { supported: true }))
}

pub(super) fn create(backend: &PeerBackend, path: &str) -> io::Result<()> {
    require_feature(backend, path)?;
    match super::reversible_replace::call_once(backend,
        FsRequest::CreateDirExclusive { path: path.into() }, "create_dir_exclusive")? {
        FsResponse::Ok => Ok(()),
        _ => Err(peer_stream::invalid("Invalid exclusive-directory reply")),
    }
}

pub(super) fn identity(backend: &PeerBackend, path: &str) -> io::Result<Option<String>> {
    require_feature(backend, path)?;
    match backend.request(FsRequest::DirectoryIdentity { path: path.into() })? {
        FsResponse::DirectoryIdentity { identity } => Ok(identity),
        _ => Err(peer_stream::invalid("Invalid directory-identity reply")),
    }
}

fn require_feature(backend: &PeerBackend, path: &str) -> io::Result<()> {
    if !peer_stream::features(backend, path)?.folder_reservation_v1 {
        return Err(io::Error::new(io::ErrorKind::Unsupported,
            "Share host does not offer atomic folder reservation"));
    }
    Ok(())
}

pub(in crate::share) fn host_create(target: ResolvedTarget, exclusive: bool, strict: bool) -> io::Result<()> {
    if strict {
        crate::vfs::create_dir_exclusive(&*target.backend, &target.path)
    } else if exclusive {
        target.backend.create_dir_new(&target.path)
    } else {
        target.backend.create_dir(&target.path)
    }
}

pub(in crate::share) fn host_supported(access: &FsAccess, path: &str) -> io::Result<bool> {
    access.check_read()?;
    let target = access.resolve(path)?;
    let supported = crate::vfs::directory_reservation_supported(&*target.backend, &target.path);
    access.check_read()?;
    let checked = access.resolve(path)?;
    if checked.mount_key != target.mount_key || checked.path != target.path
        || checked.backend.namespace_identity() != target.backend.namespace_identity() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Directory reservation export changed"));
    }
    Ok(supported)
}

pub(in crate::share) fn host_identity(access: &FsAccess, path: &str) -> io::Result<Option<String>> {
    access.check_read()?;
    let target = access.resolve(path)?;
    let identity = crate::vfs::directory_identity(&*target.backend, &target.path)?;
    access.check_read()?;
    // Object metadata must never confirm a different mount/account after a
    // provider read or an intervening export/permission change.
    let checked = access.resolve(path)?;
    if checked.mount_key != target.mount_key || checked.path != target.path
        || checked.backend.namespace_identity() != target.backend.namespace_identity() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Directory identity export changed"));
    }
    Ok(identity)
}

/// Shared authorized control boundary for both legacy mkdir and P14.
/// Moving this coherent dispatcher keeps server_fs below the source limit.
pub(in crate::share) async fn serve(
    mut send: iroh::endpoint::SendStream,
    request: FsRequest,
    access: FsAccess,
    authorization: Option<crate::share::mount_lease::MountLeaseAuthorization>,
    principal: crate::share::session::PeerPrincipal,
) -> io::Result<()> {
    let writes = request.mutates_filesystem();
    let result = crate::share::blocking::run_for(
        principal,
        crate::share::blocking::Class::Control,
        "Share directory reservation",
        move || {
            let operation = || match request {
                FsRequest::DirectoryReservationSupported { path } => {
                    host_supported(&access, &path).map(|supported|
                        FsResponse::DirectoryReservationSupported { supported })
                }
                FsRequest::DirectoryIdentity { path } => {
                    host_identity(&access, &path).map(|identity| FsResponse::DirectoryIdentity { identity })
                }
                FsRequest::CreateDir { path, exclusive } => {
                    host_create(access.resolve_write(&path)?, exclusive, false).map(|()| FsResponse::Ok)
                }
                FsRequest::CreateDirExclusive { path } => {
                    host_create(access.resolve_write(&path)?, true, true).map(|()| FsResponse::Ok)
                }
                _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Unexpected directory request")),
            };
            if writes {
                crate::share::mount_lease::run_authorized(authorization.as_ref(), operation)
            } else { operation() }
        },
    ).await;
    match result {
        Ok(response) => crate::share::framing::reply(&mut send, response).await,
        Err(error) => crate::share::framing::reply_err(&mut send, error).await,
    }
}
