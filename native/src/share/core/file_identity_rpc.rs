//! Explicit file-object evidence through the existing Share access boundary.
use crate::share::{
    backend::PeerBackend,
    fs_access::FsAccess,
    peer_stream,
    session::relation_kind_id,
    wire::{discardable_stage, FsRequest, FsResponse},
};
use sha2::{Digest, Sha256};
use std::io;

const MAX_IDENTITY: usize = 65_536;

/// A generated name is only a reason to demand a creator ticket. It never
/// establishes ownership of an existing file.
fn private_stage(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    crate::vfs::is_unique_stage(name) || discardable_stage(name)
}

/// Bind the actual export/retained lease root, not a provider instance that
/// open_saved_at may have constructed with the queried leaf as root_display.
fn root_scope(
    access: &FsAccess,
    path: &str,
    target: &crate::share::fs::ResolvedTarget,
) -> io::Result<String> {
    access.check_read()?;
    match access {
        FsAccess::Mounted(lease) => lease.file_identity_scope(),
        FsAccess::Authorized { access, .. } => root_scope(access, path, target),
        FsAccess::Dynamic(_) => {
            let parts = crate::share::fs::split_clean(path)?;
            let depth = if parts.first().is_some_and(|name| name == "Verbindungen") {
                2
            } else {
                1
            };
            if parts.len() <= depth {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "File path is missing",
                ));
            }
            let virtual_root = format!("/{}", parts[..depth].join("/"));
            let root = access.resolve(&virtual_root)?;
            if root.mount_key != target.mount_key
                || root.backend.namespace_identity() != target.backend.namespace_identity()
            {
                return Err(denied("File and export root belong to different backends"));
            }
            serde_json::to_string(&(
                virtual_root,
                root.mount_key,
                root.path,
                root.backend.namespace_identity(),
            ))
            .map_err(io::Error::other)
        }
    }
}

pub(in crate::share) fn host(access: &FsAccess, path: &str) -> io::Result<Option<String>> {
    access.check_read()?;
    let stage = private_stage(path);
    // The client's creator ticket does not replace the host's FinishStage
    // grant/private-path/root boundary. Stage facts require that same write
    // authority; ordinary source identities remain read-only metadata.
    let target = if stage {
        access.resolve_write(path)?
    } else {
        access.resolve(path)?
    };
    let scope = root_scope(access, path, &target)?;
    let identity = crate::vfs::file_identity(&*target.backend, &target.path)?;
    access.check_read()?;
    let checked = if stage {
        access.resolve_write(path)?
    } else {
        access.resolve(path)?
    };
    if scope != root_scope(access, path, &checked)?
        || checked.mount_key != target.mount_key
        || checked.path != target.path
        || checked.backend.namespace_identity() != target.backend.namespace_identity()
    {
        return Err(denied("Share file identity export changed"));
    }
    identity
        .map(|identity| {
            if identity.len() > MAX_IDENTITY {
                return Err(peer_stream::invalid("File identity exceeds protocol limit"));
            }
            let bound = serde_json::to_string(&(
                "share-file-v1",
                scope,
                &target.mount_key,
                target.backend.namespace_identity(),
                identity,
            ))
            .map_err(io::Error::other)?;
            // The proof binds physical root/account facts without disclosing
            // host-local root paths through the remote metadata interface.
            Ok(format!(
                "share-file-v1:{:x}",
                Sha256::digest(bound.as_bytes())
            ))
        })
        .transpose()
}

pub(in crate::share) async fn serve(
    mut send: iroh::endpoint::SendStream,
    path: String,
    access: FsAccess,
    authorization: Option<crate::share::mount_lease::MountLeaseAuthorization>,
    principal: crate::share::session::PeerPrincipal,
) -> io::Result<()> {
    let result = crate::share::blocking::run_for(
        principal,
        crate::share::blocking::Class::Control,
        "Share file identity",
        move || {
            crate::share::mount_lease::run_authorized(authorization.as_ref(), || {
                host(&access, &path).map(|identity| FsResponse::FileIdentity { identity })
            })
        },
    )
    .await;
    match result {
        Ok(response) => crate::share::framing::reply(&mut send, response).await,
        Err(error) => crate::share::framing::reply_err(&mut send, error).await,
    }
}

fn peer_binding(backend: &PeerBackend) -> io::Result<String> {
    let endpoint = backend.current_endpoint()?;
    let (kind, relation) = relation_kind_id(&endpoint);
    serde_json::to_string(&(kind, relation, &endpoint.presence.node_id)).map_err(io::Error::other)
}

pub(super) fn client(backend: &PeerBackend, path: &str) -> io::Result<Option<String>> {
    let stage = private_stage(path);
    if stage {
        // A user may deliberately select a normal file with a generated-
        // looking literal name. Unknown identity preserves that read path
        // while withholding facts about a stage this backend does not own.
        if !backend.has_stage_ticket(path)? {
            return Ok(None);
        }
        backend.verify_owned_stage(path)?;
    }
    if !peer_stream::features(backend, path)?.file_identity_v1 {
        return Ok(None);
    }
    let binding = peer_binding(backend)?;
    let lease = backend.mount_lease_token()?;
    let identity = match backend.request(FsRequest::FileIdentity { path: path.into() })? {
        FsResponse::FileIdentity { identity } => identity,
        _ => return Err(peer_stream::invalid("Invalid file-identity reply")),
    };
    if binding != peer_binding(backend)? || lease != backend.mount_lease_token()? {
        return Err(denied("Share file identity authority changed"));
    }
    if stage {
        // Read-only: retain the same Ready ticket for FinishStage/promotion.
        backend.verify_owned_stage(path)?;
    }
    identity
        .filter(|id| !id.is_empty())
        .map(|identity| {
            if identity.len() > MAX_IDENTITY {
                return Err(peer_stream::invalid("File identity exceeds protocol limit"));
            }
            serde_json::to_string(&("peer-file-v1", binding, identity)).map_err(io::Error::other)
        })
        .transpose()
}

fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
