//! Filesystem requests of one authorized Share stream. Short control
//! operations run on the shared blocking pool; transfers take an admission
//! slot first; batch entries are admitted one by one like single requests.
use std::io;
use std::sync::{Arc, Mutex};

use iroh::endpoint::{RecvStream, SendStream};

use crate::share::core::eio;
use crate::share::framing::{reply, reply_err};
use crate::share::fs::ResolvedTarget;
use crate::share::fs_access::FsAccess;
use crate::share::mount_lease::{run_authorized, MountLeaseAuthorization};
use crate::share::node::ShareIrohNode;
use crate::share::server_transfer::{ReadSource, WriteMode};
use crate::share::session::{IncomingSession, PeerPrincipal};
use crate::share::types::ShareAuthState;
use crate::share::wire::{discardable_stage, validate_get, validate_put, FsRequest, FsResponse};

use super::batch_status::{self, BatchKey};
use super::StreamContext;

/// One authorized filesystem stream, ready for its request.
pub(super) struct FsStream {
    pub(super) send: SendStream,
    pub(super) recv: RecvStream,
    pub(super) access: FsAccess,
    /// Lease re-admission for mutations and batch entries of mounted streams.
    pub(super) authorization: Option<MountLeaseAuthorization>,
    pub(super) context: StreamContext,
    pub(super) principal: PeerPrincipal,
}

/// Admits each batch entry exactly like one single request: Share still
/// active, the current grant and exports (or the mount lease), then the path.
pub(super) struct BatchAuthority {
    access: FsAccess,
    lease: Option<MountLeaseAuthorization>,
    session: Arc<IncomingSession>,
    auth: Arc<Mutex<ShareAuthState>>,
    node: Arc<ShareIrohNode>,
}

impl BatchAuthority {
    fn new(
        access: FsAccess,
        lease: Option<MountLeaseAuthorization>,
        context: &StreamContext,
    ) -> Self {
        Self {
            access,
            lease,
            session: context.session.clone(),
            auth: context.auth.clone(),
            node: context.node.clone(),
        }
    }

    pub(super) fn admit<T>(
        &self,
        operation: impl FnOnce(&FsAccess) -> io::Result<T>,
    ) -> io::Result<T> {
        if let Some(lease) = &self.lease {
            return lease.run(|| operation(&self.access));
        }
        self.node.require_sharing_active()?;
        let exports = self.session.authorize(&self.auth)?;
        operation(&FsAccess::dynamic(exports))
    }
}

pub(super) async fn serve(stream: FsStream, req: FsRequest) -> io::Result<()> {
    let FsStream {
        mut send,
        recv,
        access,
        authorization,
        context,
        principal,
    } = stream;
    if req.is_transfer_v1() && context.node.legacy_transfer_host() {
        // What a host before transfer v1 does: the request does not parse.
        return Err(eio("unbekannte Dateioperation"));
    }
    match req {
        FsRequest::Capabilities { .. } => Err(eio("Capabilities wurden doppelt verarbeitet")),
        FsRequest::ReleaseLease => Err(eio("Lease-Freigabe wurde doppelt verarbeitet")),
        FsRequest::WriteDone => reply_err(&mut send, eio("unerwartetes Schreib-Ende")).await,
        FsRequest::ListDir { path } => {
            match control("Share list directory", move || access.list_dir(&path)).await {
                Ok(entries) => reply(&mut send, FsResponse::Entries { entries }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::Stat { path } => match control("Share stat", move || access.stat(&path)).await {
            Ok(meta) => reply(&mut send, FsResponse::Meta { meta }).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::WalkTree { path } => crate::share::walk::serve_walk(send, path, access).await,
        FsRequest::StorageSnapshot { path } => {
            crate::share::storage_snapshot::serve_snapshot(send, path, access).await
        }
        FsRequest::StorageAnalysis(request) => {
            crate::share::storage_analysis_server::serve(send, request, access, principal).await
        }
        FsRequest::Read { path } => {
            let source = ReadSource {
                path,
                id: None,
                offset: 0,
            };
            read(send, &context, source, access).await
        }
        FsRequest::ReadAt { path, id, offset } => {
            let source = ReadSource { path, id, offset };
            read(send, &context, source, access).await
        }
        FsRequest::Write { path } => {
            write(
                send,
                recv,
                &context,
                (path, WriteMode::Replace),
                access,
                authorization,
            )
            .await
        }
        FsRequest::WriteNew { path } => {
            write(
                send,
                recv,
                &context,
                (path, WriteMode::Create),
                access,
                authorization,
            )
            .await
        }
        FsRequest::MkdirAll { path } => {
            simple(
                &mut send,
                path,
                access,
                authorization,
                "Share create directory",
                |target| target.backend.mkdir_all(&target.path),
            )
            .await
        }
        FsRequest::CreateDir { path, exclusive } => {
            simple(
                &mut send,
                path,
                access,
                authorization,
                "Share create one directory",
                move |target| {
                    if exclusive {
                        target.backend.create_dir_new(&target.path)
                    } else {
                        target.backend.create_dir(&target.path)
                    }
                },
            )
            .await
        }
        FsRequest::Rename { src, dst } => {
            let result = control("Share rename", move || {
                run_authorized(authorization.as_ref(), || access.rename(&src, &dst, false))
            })
            .await;
            reply_unit(&mut send, result).await
        }
        FsRequest::RenameNoReplace { src, dst } => {
            let result = control("Share no-replace rename", move || {
                run_authorized(authorization.as_ref(), || access.rename(&src, &dst, true))
            })
            .await;
            reply_unit(&mut send, result).await
        }
        FsRequest::PromoteStaged {
            staged,
            destination,
        } => {
            let result = control("Share promote staged file", move || {
                run_authorized(authorization.as_ref(), || {
                    access.promote_staged(&staged, &destination)
                })
            })
            .await;
            reply_unit(&mut send, result).await
        }
        FsRequest::PromoteNoReplace {
            staged,
            destination,
            copy,
        } => {
            let result = control("Share publish stage", move || {
                run_authorized(authorization.as_ref(), || {
                    access.promote_no_replace(&staged, &destination, copy)
                })
            })
            .await;
            reply_unit(&mut send, result).await
        }
        FsRequest::DiscardStage { path } => {
            if !discardable_stage_path(&path) {
                // Only the transfer engine's own upload stages; never a user
                // file or a stage the host keeps (K17).
                let refused = io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Nur private Upload-Stufen können verworfen werden",
                );
                return reply_err(&mut send, refused).await;
            }
            simple(
                &mut send,
                path,
                access,
                authorization,
                "Share discard stage",
                |target| target.backend.discard_copy_stage(&target.path),
            )
            .await
        }
        FsRequest::CopyFile { src, dst } => {
            let admitted = context.admit(&send).await;
            let slot = match admitted {
                Ok(slot) => slot,
                Err(error) => return reply_err(&mut send, error).await,
            };
            crate::share::fs_copy::serve(&mut send, src, dst, access, authorization, slot).await
        }
        FsRequest::RemoveFile { path } => {
            simple(
                &mut send,
                path,
                access,
                authorization,
                "Share remove file",
                |target| target.backend.remove_file(&target.path),
            )
            .await
        }
        FsRequest::RemoveDir { path } => {
            simple(
                &mut send,
                path,
                access,
                authorization,
                "Share remove directory",
                |target| crate::share::fs::remove_dir_recursive(&*target.backend, &target.path),
            )
            .await
        }
        FsRequest::PutBatch { nonce, entries } => {
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
            let job = super::batch_put::PutBatchJob {
                key: BatchKey::new(principal, nonce.clone()),
                nonce,
                entries,
                authority: BatchAuthority::new(access, authorization, &context),
                expected_lease,
            };
            super::batch_put::serve(send, recv, job, slot, context.stall()).await
        }
        FsRequest::PutBatchStatus { nonce } => match batch_status::query(&principal, &nonce) {
            Some(status) => reply(&mut send, FsResponse::Batch { status }).await,
            None => {
                let unknown = io::Error::new(
                    io::ErrorKind::NotFound,
                    "Paket ist dem Host unbekannt; sein Ergebnis bleibt offen",
                );
                reply_err(&mut send, unknown).await
            }
        },
        FsRequest::GetBatch { items } => {
            if let Err(error) = validate_get(&items) {
                return reply_err(&mut send, error).await;
            }
            let admitted = context.admit(&send).await;
            let slot = match admitted {
                Ok(slot) => slot,
                Err(error) => return reply_err(&mut send, error).await,
            };
            let authority = BatchAuthority::new(access, authorization, &context);
            super::batch_get::serve(send, items, authority, slot, context.stall()).await
        }
        FsRequest::DuplicateSearch(request) => {
            crate::share::host_requests::serve_duplicate_search(send, request, access, principal)
                .await
        }
        FsRequest::HashWalk(request) => {
            crate::share::host_requests::serve_hash_walk(send, request, access, principal).await
        }
        FsRequest::ListDirBatch(request) => {
            crate::share::host_requests::serve_list_batch(send, request, access, principal).await
        }
        FsRequest::WatchExport(request) => {
            crate::share::host_requests::serve_watch(send, request, access, principal).await
        }
        FsRequest::Recycle(request) => {
            let path = request.path.clone();
            answer(
                &mut send,
                path,
                access,
                authorization,
                "Share recycle",
                move |target| crate::share::host_requests::serve_recycle(target, request),
            )
            .await
        }
        FsRequest::FinishStage(request) => {
            let path = request.staged.clone();
            answer(
                &mut send,
                path,
                access,
                authorization,
                "Share finish stage",
                move |target| crate::share::host_requests::serve_finish_stage(target, request),
            )
            .await
        }
        FsRequest::SyncFilesystem(request) => {
            let path = request.path.clone();
            answer(
                &mut send,
                path,
                access,
                authorization,
                "Share sync filesystem",
                move |target| crate::share::host_requests::serve_sync_filesystem(target, request),
            )
            .await
        }
    }
}

/// Whether `path` names a stage a client may have the host discard.
fn discardable_stage_path(path: &str) -> bool {
    match crate::share::fs_paths::split_clean(path) {
        Ok(parts) => parts.last().is_some_and(|name| discardable_stage(name)),
        Err(_) => false,
    }
}

async fn read(
    mut send: SendStream,
    context: &StreamContext,
    source: ReadSource,
    access: FsAccess,
) -> io::Result<()> {
    let admitted = context.admit(&send).await;
    let slot = match admitted {
        Ok(slot) => slot,
        Err(error) => return reply_err(&mut send, error).await,
    };
    crate::share::server_transfer::read_file(send, source, access, slot, context.stall()).await
}

async fn write(
    mut send: SendStream,
    recv: RecvStream,
    context: &StreamContext,
    (path, mode): (String, WriteMode),
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
) -> io::Result<()> {
    let admitted = context.admit(&send).await;
    let slot = match admitted {
        Ok(slot) => slot,
        Err(error) => return reply_err(&mut send, error).await,
    };
    let target = crate::share::server_transfer::WriteTarget {
        path,
        mode,
        authorization,
    };
    crate::share::server_transfer::write_file(send, recv, target, access, slot, context.stall())
        .await
}

async fn simple<F>(
    send: &mut SendStream,
    path: String,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
    label: &'static str,
    operation: F,
) -> io::Result<()>
where
    F: FnOnce(ResolvedTarget) -> io::Result<()> + Send + 'static,
{
    let result = control(label, move || {
        run_authorized(authorization.as_ref(), || {
            access.resolve(&path).and_then(operation)
        })
    })
    .await;
    reply_unit(send, result).await
}

/// Like `simple`, for an operation that builds its own reply.
async fn answer<F>(
    send: &mut SendStream,
    path: String,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
    label: &'static str,
    operation: F,
) -> io::Result<()>
where
    F: FnOnce(ResolvedTarget) -> io::Result<FsResponse> + Send + 'static,
{
    let result = control(label, move || {
        run_authorized(authorization.as_ref(), || {
            access.resolve(&path).and_then(operation)
        })
    })
    .await;
    match result {
        Ok(response) => reply(send, response).await,
        Err(error) => reply_err(send, error).await,
    }
}

async fn reply_unit(send: &mut SendStream, result: io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => reply(send, FsResponse::Ok).await,
        Err(error) => reply_err(send, error).await,
    }
}

async fn control<T, F>(label: &'static str, operation: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    crate::share::blocking::run(label, operation).await
}
