//! Reversible publication through one live export/mount authority.
use std::io;
use iroh::endpoint::SendStream;
use crate::share::{
    blocking::{self, Class},
    framing::{reply, reply_err},
    fs_access::FsAccess,
    fs_paths::split_clean,
    fs_request::FsReversibleReplace,
    mount_lease::{run_authorized, MountLeaseAuthorization},
    session::PeerPrincipal,
    wire::FsResponse,
};

/// Compare virtual names only; never decode or rewrite provider locators.
pub(in crate::share) fn validate(request: &FsReversibleReplace) -> io::Result<()> {
    let staged = split_clean(&request.staged)?;
    let destination = split_clean(&request.destination)?;
    let retained = split_clean(&request.retained)?;
    if [staged.len(), destination.len(), retained.len()]
        .iter()
        .any(|length| *length < 2)
    {
        return Err(invalid());
    }
    let parent = &destination[..destination.len() - 1];
    let name = retained.last().ok_or_else(invalid)?;
    let nonce = name.strip_prefix(".se-replace-").ok_or_else(invalid)?;
    if &staged[..staged.len() - 1] != parent
        || &retained[..retained.len() - 1] != parent
        || staged == destination
        || retained == staged
        || retained == destination
        || nonce.len() != 16
        || !nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !crate::vfs::is_staging_name(staged.last().ok_or_else(invalid)?)
    {
        return Err(invalid());
    }
    Ok(())
}

impl FsAccess {
    pub(in crate::share) fn replace_staged_reversible(
        &self,
        request: &FsReversibleReplace,
    ) -> io::Result<bool> {
        validate(request)?;
        self.check_write()?;
        let stage = self.resolve_write(&request.staged)?;
        let destination = self.resolve_write(&request.destination)?;
        let retained = self.resolve_write(&request.retained)?;
        self.require_same_backend(&stage, &destination)?;
        self.require_same_backend(&stage, &retained)?;
        // Each resolved target retains the same current authority. The guard
        // rechecks all paths immediately before the real provider hook.
        let replaced = crate::vfs::replace_staged_reversible(
            &*stage.backend,
            &stage.path,
            &destination.path,
            &retained.path,
        )?;
        self.check_write()?;
        Ok(replaced)
    }
}

pub(in crate::share) async fn serve(
    mut send: SendStream,
    request: FsReversibleReplace,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let authority = access.clone();
    let result = blocking::run_for(
        principal,
        Class::Control,
        "Share reversible replace",
        move || {
            run_authorized(authorization.as_ref(), || {
                access.replace_staged_reversible(&request)
            })
        },
    )
    .await;
    let result = result.and_then(|replaced| {
        authority.check_write()?;
        Ok(replaced)
    });
    match result {
        Ok(replaced) => reply(&mut send, FsResponse::ReversibleReplaced { replaced }).await,
        Err(error) => reply_err(&mut send, error).await,
    }
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Ersetzung braucht verschiedene Stage/Zielpfade und einen .se-replace-16lowerhex-Sibling",
    )
}
