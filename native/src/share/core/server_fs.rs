//! Filesystem requests of one authorized Share stream. Short control
//! operations run on the shared blocking pool; transfers take an admission
//! slot first; batch entries are admitted one by one like single requests.
use std::io;
use std::sync::{Arc, Mutex};

use iroh::endpoint::{RecvStream, SendStream};

use crate::share::core::eio;
use crate::share::framing::{reply, reply_err};
use crate::share::fs_access::FsAccess;
use crate::share::mount_lease::{run_authorized, MountLeaseAuthorization};
use crate::share::node::ShareIrohNode;
use crate::share::server_transfer::{ReadSource, WriteMode};
use crate::share::session::{IncomingSession, PeerPrincipal};
use crate::share::types::ShareAuthState;
use crate::share::wire::{discardable_stage, validate_get, validate_put, FsRequest, FsResponse};

use super::batch_status::{self, BatchKey};
use super::StreamContext;
#[path = "server_fs_control.rs"]
mod control_operations;
use control_operations::{answer, control, reply_unit, simple};

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
    pub(super) fn stream_access(&self) -> FsAccess { self.access.clone() }
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
        self.session.authorize(&self.auth)?;
        self.access.check_read()?;
        operation(&self.access)
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
        FsRequest::ReplaceStagedReversible(request) => {
            crate::share::fs_access::reversible_replace::serve(
                send, request, access, authorization, principal,
            ).await
        }
        FsRequest::WriteDone => reply_err(&mut send, eio("unerwartetes Schreib-Ende")).await,
        FsRequest::ListDir { path } => {
            match control(&principal, "Share list directory", move || access.list_dir(&path)).await {
                Ok(entries) => reply(&mut send, FsResponse::Entries { entries }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::Stat { path } => match control(&principal, "Share stat", move || access.stat(&path)).await {
            Ok(meta) => reply(&mut send, FsResponse::Meta { meta }).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::SyncChildPath { parent, literal_name } => {
            let result = control(&principal, "Share literal child", move ||
                crate::share::peer_extensions::literal_paths::host(&access, &parent, &literal_name)).await;
            match result {
                Ok(path) => reply(&mut send, FsResponse::ChildPath { path }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::WalkTree { path } => crate::share::walk::serve_walk_for(send, path, access, principal).await,
        FsRequest::StorageSnapshot { path } => {
            crate::share::storage_snapshot::serve_snapshot(send, path, access, principal).await
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
                &principal,
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
                &principal,
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
            let result = control(&principal, "Share rename", move || {
                run_authorized(authorization.as_ref(), || access.rename(&src, &dst, false))
            })
            .await;
            reply_unit(&mut send, result).await
        }
        FsRequest::RenameNoReplace { src, dst } => {
            let result = control(&principal, "Share no-replace rename", move || {
                run_authorized(authorization.as_ref(), || access.rename(&src, &dst, true))
            })
            .await;
            reply_unit(&mut send, result).await
        }
        FsRequest::PromoteStaged {
            staged,
            destination,
        } => {
            let result = control(&principal, "Share promote staged file", move || {
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
            let result = control(&principal, "Share publish stage", move || {
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
                &principal,
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
                &principal,
                "Share remove file",
                |target| target.backend.remove_file(&target.path),
            )
            .await
        }
        FsRequest::RemoveDir { path } => {
            let live = access.clone();
            simple(
                &mut send,
                path,
                access,
                authorization,
                &principal,
                "Share remove directory",
                move |target| {
                    crate::share::fs::require_target_destructive(&target)?;
                    crate::share::fs_delete::remove_tree_checked(&*target.backend, &target.path, &|| live.check_write())
                },
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
                &principal,
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
                &principal,
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
                &principal,
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
