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
use crate::share::wire::{discardable_stage, FsRequest, FsResponse};

use super::StreamContext;
#[path = "server_fs_io.rs"]
mod streams;
use streams::{read, write};
#[path = "server_fs_control.rs"]
mod control_operations;
use control_operations::{answer, control, reply_unit, simple};
#[path = "server_fs_batches.rs"]
mod batches;
#[path = "server_stage_ranges.rs"]
pub(super) mod stage_ranges;

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
    pub(super) fn stream_access(&self) -> FsAccess {
        self.access.clone()
    }
    /// This access without its stream marker (`FsAccess::retained_snapshot`),
    /// taken while the request is alive; `None` once it ended.
    pub(super) fn retained_access(&self) -> Option<FsAccess> {
        self.access.retained_snapshot().ok()
    }
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
        // The grant still authorizes the session (Z-H1: no export copy).
        self.session.may_write(&self.auth)?;
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
        FsRequest::FileIdentity { path } => {
            crate::share::peer_extensions::file_identity::serve(
                send,
                path,
                access,
                authorization,
                principal,
            )
            .await
        }
        FsRequest::Capacity { path } => {
            let result = control(&principal, "Share capacity", move || {
                crate::share::capacity::host(&access, &path)
            })
            .await;
            match result {
                Ok(capacity) => reply(&mut send, FsResponse::Capacity { capacity }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::Capabilities { .. } => Err(eio("Capabilities wurden doppelt verarbeitet")),
        FsRequest::ReleaseLease => Err(eio("Lease-Freigabe wurde doppelt verarbeitet")),
        FsRequest::ReplaceStagedReversible(request) => {
            crate::share::fs_access::reversible_replace::serve(
                send,
                request,
                access,
                authorization,
                principal,
            )
            .await
        }
        FsRequest::WriteDone => reply_err(&mut send, eio("unerwartetes Schreib-Ende")).await,
        FsRequest::ListDir { path } => {
            match control(&principal, "Share list directory", move || {
                access.list_dir(&path)
            })
            .await
            {
                Ok(entries) => reply(&mut send, FsResponse::Entries { entries }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::Stat { path } => {
            match control(&principal, "Share stat", move || access.stat(&path)).await {
                Ok(meta) => reply(&mut send, FsResponse::Meta { meta }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::SyncChildPath {
            parent,
            literal_name,
        } => {
            let result = control(&principal, "Share literal child", move || {
                crate::share::peer_extensions::literal_paths::host(&access, &parent, &literal_name)
            })
            .await;
            match result {
                Ok(path) => reply(&mut send, FsResponse::ChildPath { path }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::WalkTree { path } => {
            crate::share::walk::serve_walk_for(send, path, access, principal).await
        }
        FsRequest::StorageSnapshot { path } => {
            crate::share::storage_snapshot::serve_snapshot(send, path, access, principal).await
        }
        FsRequest::StorageAnalysis(request) => {
            crate::share::storage_analysis_server::serve(send, request, access, principal).await
        }
        FsRequest::Read { path, lz4 } => {
            let source = ReadSource {
                path,
                id: None,
                offset: 0,
                len: 0,
                lz4,
                evidence: false,
            };
            read(send, &context, source, access).await
        }
        FsRequest::ReadAt {
            path,
            id,
            offset,
            len,
            lz4,
            evidence,
        } => {
            let source = ReadSource {
                path,
                id,
                offset,
                len,
                lz4,
                evidence,
            };
            read(send, &context, source, access).await
        }
        FsRequest::Write { path, lz4 } => {
            write(
                send,
                recv,
                &context,
                (path, WriteMode::Replace, lz4, None),
                (access, None),
                authorization,
            )
            .await
        }
        FsRequest::WriteNew {
            path,
            lz4,
            evidence,
            probe,
            mtime_ms,
        } => {
            write(
                send,
                recv,
                &context,
                (path, WriteMode::Create, lz4, evidence.then_some(probe)),
                (access, mtime_ms),
                authorization,
            )
            .await
        }
        request @ (FsRequest::SegmentedStageOpen(_)
        | FsRequest::StageWriteAt(_)
        | FsRequest::SegmentedStageFinish(_)) => {
            let stream = FsStream {
                send,
                recv,
                access,
                authorization,
                context,
                principal,
            };
            stage_ranges::serve(stream, request).await
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
        request @ (FsRequest::CreateDir { .. }
        | FsRequest::RenameReservedDirectory { .. }
        | FsRequest::CreateDirExclusive { .. }
        | FsRequest::DirectoryIdentity { .. }
        | FsRequest::DirectoryReservationSupported { .. }) => {
            crate::share::peer_extensions::folder_reservation::serve(
                send,
                request,
                access,
                authorization,
                principal,
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
            evidence,
        } => {
            let stream = FsStream {
                send,
                recv,
                access,
                authorization,
                context,
                principal,
            };
            batches::promote(stream, staged, destination, copy, evidence).await
        }
        FsRequest::CreateDirs(request) => {
            let stream = FsStream {
                send,
                recv,
                access,
                authorization,
                context,
                principal,
            };
            batches::create_dirs(stream, request).await
        }
        FsRequest::DiscardStage { path } if context.stages.holds(&path) => {
            // A stage of a ranged upload this connection still has open: the
            // table closes its handle and removes the file.
            stage_ranges::discard(send, path, access, authorization, &context, &principal).await
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
                    crate::share::fs_delete::remove_tree_checked(
                        &*target.backend,
                        &target.path,
                        &|| live.check_write(),
                    )
                },
            )
            .await
        }
        FsRequest::RemoveTree { path } => {
            crate::share::host_remove_tree::serve(send, path, access, authorization, principal)
                .await
        }
        request @ (FsRequest::PutBatch { .. }
        | FsRequest::PutBatchStatus { .. }
        | FsRequest::GetBatch { .. }) => {
            let stream = FsStream {
                send,
                recv,
                access,
                authorization,
                context,
                principal,
            };
            match request {
                FsRequest::PutBatch {
                    nonce,
                    entries,
                    evidence,
                } => batches::put_batch(stream, nonce, entries, evidence).await,
                FsRequest::GetBatch { items, evidence } => {
                    batches::get_batch(stream, items, evidence).await
                }
                FsRequest::PutBatchStatus { nonce } => {
                    batches::put_batch_status(stream, nonce).await
                }
                _ => Err(eio("Paketanfrage falsch zugeordnet")),
            }
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
