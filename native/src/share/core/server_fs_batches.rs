//! Transfer v1 requests of one authorized Share stream that work on many
//! files or report what the host read: PutBatch, PutBatchStatus, GetBatch,
//! PromoteNoReplace (with `evidence_v1`) and CreateDirs (`create_dirs_v1`).
use std::io;

use crate::share::framing::{reply, reply_err};
use crate::share::fs_access::FsAccess;
use crate::share::host_evidence::{self, Scopes};
use crate::share::mount_lease::run_authorized;
use crate::share::wire::{
    validate_create_dirs, validate_get, validate_put, FsBatchGet, FsBatchPut, FsCreateDirs,
    FsDirOutcome, FsResponse,
};

use super::super::batch_status::{self, BatchKey};
use super::control_operations::control;
use super::{BatchAuthority, FsStream};

pub(super) async fn put_batch(
    stream: FsStream,
    nonce: String,
    entries: Vec<FsBatchPut>,
    evidence: bool,
) -> io::Result<()> {
    let FsStream {
        mut send,
        recv,
        access,
        authorization,
        context,
        principal,
    } = stream;
    if let Err(error) = validate_put(&nonce, &entries) {
        return reply_err(&mut send, error).await;
    }
    let admitted = context.admit(&send).await;
    let slot = match admitted {
        Ok(slot) => slot,
        Err(error) => return reply_err(&mut send, error).await,
    };
    let expected_lease = authorization
        .as_ref()
        .map(|authorization| authorization.token().to_string());
    let job = super::super::batch_put::PutBatchJob {
        key: BatchKey::new(principal, nonce.clone()),
        nonce,
        entries,
        authority: BatchAuthority::new(access, authorization, &context),
        expected_lease,
        evidence,
    };
    super::super::batch_put::serve(send, recv, job, slot, context.stall()).await
}

pub(super) async fn put_batch_status(stream: FsStream, nonce: String) -> io::Result<()> {
    let FsStream {
        mut send,
        principal,
        ..
    } = stream;
    match batch_status::query(&principal, &nonce) {
        Some(status) => reply(&mut send, FsResponse::Batch { status }).await,
        None => {
            let unknown = io::Error::new(
                io::ErrorKind::NotFound,
                "Paket ist dem Host unbekannt; sein Ergebnis bleibt offen",
            );
            reply_err(&mut send, unknown).await
        }
    }
}

pub(super) async fn get_batch(
    stream: FsStream,
    items: Vec<FsBatchGet>,
    evidence: bool,
) -> io::Result<()> {
    let FsStream {
        mut send,
        access,
        authorization,
        context,
        ..
    } = stream;
    if let Err(error) = validate_get(&items) {
        return reply_err(&mut send, error).await;
    }
    let admitted = context.admit(&send).await;
    let slot = match admitted {
        Ok(slot) => slot,
        Err(error) => return reply_err(&mut send, error).await,
    };
    let authority = BatchAuthority::new(access, authorization, &context);
    super::super::batch_get::serve(send, items, authority, slot, context.stall(), evidence).await
}

/// PromoteNoReplace; with `evidence` the reply is `Published` with the
/// published file as the host reads it back (absent when it cannot, the
/// publication stands either way).
pub(super) async fn promote(
    stream: FsStream,
    staged: String,
    destination: String,
    copy: bool,
    evidence: bool,
) -> io::Result<()> {
    let FsStream {
        mut send,
        access,
        authorization,
        principal,
        ..
    } = stream;
    let result = control(&principal, "Share publish stage", move || {
        run_authorized(authorization.as_ref(), || {
            access.promote_no_replace(&staged, &destination, copy)?;
            if !evidence {
                return Ok(FsResponse::Ok);
            }
            let evidence = access.resolve(&destination).ok().and_then(|target| {
                host_evidence::of_target(&access, &destination, &target, &mut Scopes::default())
            });
            Ok(FsResponse::Published { evidence })
        })
    })
    .await;
    match result {
        Ok(response) => reply(&mut send, response).await,
        Err(error) => reply_err(&mut send, error).await,
    }
}

/// CreateDirs: each folder like a non-exclusive CreateDir, in request order,
/// reporting whether this request created it.
pub(super) async fn create_dirs(stream: FsStream, request: FsCreateDirs) -> io::Result<()> {
    let FsStream {
        mut send,
        access,
        authorization,
        principal,
        ..
    } = stream;
    if let Err(error) = validate_create_dirs(&request) {
        return reply_err(&mut send, error).await;
    }
    let result = control(&principal, "Share create directories", move || {
        run_authorized(authorization.as_ref(), || {
            let outcomes = request
                .paths
                .iter()
                .map(|path| FsDirOutcome::of(create_one(&access, path)))
                .collect();
            Ok(FsResponse::Dirs { outcomes })
        })
    })
    .await;
    match result {
        Ok(response) => reply(&mut send, response).await,
        Err(error) => reply_err(&mut send, error).await,
    }
}

/// `true` = created now. A taken name is merged exactly like `CreateDir`
/// (`create_dir`, which refuses anything but a plain folder).
fn create_one(access: &FsAccess, path: &str) -> io::Result<bool> {
    let target = access.resolve_write(path)?;
    match target.backend.create_dir_new(&target.path) {
        Ok(()) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::AlreadyExists | io::ErrorKind::Unsupported
            ) =>
        {
            target.backend.create_dir(&target.path).map(|()| false)
        }
        Err(error) => Err(error),
    }
}
