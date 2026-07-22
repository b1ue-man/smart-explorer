use std::io;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};

use super::connection_events::ConnectionErrorKind;
use super::core::eio;
use super::framing::{
    recv_ctrl_limited, reply, reply_err, send_ctrl, MAX_HANDSHAKE_CTRL_FRAME,
    MAX_REQUEST_CTRL_FRAME,
};
use super::fs::{self, ShareExportConfig};
use super::fs_access::FsAccess;
use super::handshake_limits::ApplicationHandshakePermit;
use super::io_deadline;
use super::mount_lease::{run_authorized, MountLeaseAuthorization, PeerMountLeases};
use super::node::ShareIrohNode;
use super::session::{authenticate_incoming_session, IncomingSession, PeerPrincipal};
use super::types::{ExecRequest, ShareEvent};
use super::wire::{Ctrl, FsRequest, FsResponse, MOUNT_PATH_CAPABILITY_CONTRACT_VERSION};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) async fn handle_connection(
    node: Arc<ShareIrohNode>,
    conn: Connection,
    handshake_permit: ApplicationHandshakePermit,
) -> io::Result<()> {
    let _incoming = node.track_incoming(&conn)?;
    node.require_sharing_active()?;
    let remote_node = conn.remote_id().to_string();
    let handshake_deadline = tokio::time::Instant::now() + HANDSHAKE_TIMEOUT;
    let (mut send, mut recv) = tokio::time::timeout_at(handshake_deadline, conn.accept_bi())
        .await
        .map_err(|_| eio("Session-Handshake Timeout"))?
        .map_err(eio)?;
    let hello = match io_deadline::run_until(
        handshake_deadline,
        "Session-Hello Timeout",
        recv_ctrl_limited(&mut recv, MAX_HANDSHAKE_CTRL_FRAME),
    )
    .await?
    {
        Ctrl::PeerHello { hello } => hello,
        _ => return Err(eio("Session-Hello fehlt")),
    };
    if hello.protocol_version != 3 {
        io_deadline::run_until(
            handshake_deadline,
            "Session-Ablehnung Timeout",
            send_ctrl(
                &mut send,
                &Ctrl::FsResp {
                    resp: super::fs_error::message("Inkompatibles Share-Protokoll"),
                },
            ),
        )
        .await?;
        return Err(eio("Inkompatibles Share-Protokoll"));
    }
    let session = match authenticate_incoming_session(&hello, &remote_node, &node.auth) {
        Ok(session) => session,
        Err(error) => {
            io_deadline::run_until(
                handshake_deadline,
                "Session-Ablehnung Timeout",
                send_ctrl(
                    &mut send,
                    &Ctrl::FsResp {
                        resp: super::fs_error::response(&error),
                    },
                ),
            )
            .await?;
            return Err(error);
        }
    };
    io_deadline::run_until(
        handshake_deadline,
        "Session-Bestaetigung Timeout",
        send_ctrl(&mut send, &Ctrl::PeerHelloOk),
    )
    .await?;
    drop(handshake_permit);
    let _ = node.ev.try_send(ShareEvent::Status(format!(
        "Iroh-Session akzeptiert: {} ({})",
        hello.device_id, remote_node
    )));
    let session = Arc::new(session);
    let exec_slots = super::exec::peer_slots(&remote_node);
    let legacy_connection = conn.stable_id();
    let _legacy_cleanup = super::mount_lease_cleanup::LegacyLeaseCleanup::new(
        node.mount_leases.clone(),
        legacy_connection,
        node.rt.clone(),
    );
    loop {
        let (send, recv) = match conn.accept_bi().await {
            Ok(streams) => streams,
            Err(error) => return Err(eio(error)),
        };
        let session = session.clone();
        let auth = node.auth.clone();
        let exec_slots = exec_slots.clone();
        let node = node.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_peer_stream(
                send,
                recv,
                session,
                auth,
                node.clone(),
                exec_slots,
                legacy_connection,
            )
            .await
            {
                node.emit_connection_error(ConnectionErrorKind::FsStream, error.to_string());
            }
        });
    }
}

async fn handle_peer_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    session: Arc<IncomingSession>,
    auth: Arc<Mutex<super::types::ShareAuthState>>,
    node: Arc<ShareIrohNode>,
    exec_slots: Arc<AtomicUsize>,
    legacy_connection: usize,
) -> io::Result<()> {
    if let Err(error) = session.authorize(&auth) {
        return io_deadline::run("Share authorization rejection", reply_err(&mut send, error))
            .await;
    }
    let ctrl = io_deadline::run(
        "Share operation frame",
        recv_ctrl_limited(&mut recv, MAX_REQUEST_CTRL_FRAME),
    )
    .await?;
    node.require_sharing_active()?;
    let exports = match session.authorize(&auth) {
        Ok(exports) => exports,
        Err(error) => {
            return io_deadline::run("Share authorization rejection", async {
                match ctrl {
                    Ctrl::Exec { .. } => {
                        send_ctrl(
                            &mut send,
                            &Ctrl::ExecErr {
                                msg: error.to_string(),
                            },
                        )
                        .await
                    }
                    _ => reply_err(&mut send, error).await,
                }
            })
            .await;
        }
    };
    let (req, requested_lease) = match ctrl {
        Ctrl::Fs { req, lease } => (req, lease),
        Ctrl::Exec { req } => return handle_exec_stream(&mut send, req, exec_slots).await,
        _ => return Err(eio("Dateioperation erwartet")),
    };
    let principal = session.principal();
    let mount_leases = node.mount_leases.clone();
    let req = match req {
        FsRequest::Capabilities {
            path,
            acquire_lease,
            lease_request_id,
        } => {
            return handle_capabilities(
                &mut send,
                path,
                acquire_lease,
                exports,
                principal,
                lease_request_id,
                legacy_connection,
                node.filesystem_authorization_epoch(),
                mount_leases,
            )
            .await;
        }
        FsRequest::ReleaseLease => {
            let Some(token) = requested_lease.as_deref() else {
                return reply_err(&mut send, eio("Peer-Mount-Lease fehlt bei Freigabe")).await;
            };
            let token = token.to_string();
            let result = blocking_fs("Share release mount lease", move || {
                let removed = mount_leases.release(&token, &principal)?;
                let existed = removed.is_some();
                drop(removed);
                Ok(existed)
            })
            .await;
            return match result {
                Ok(_) => reply(&mut send, FsResponse::Ok).await,
                Err(error) => reply_err(&mut send, error).await,
            };
        }
        req => req,
    };
    if matches!(&req, FsRequest::WriteDone) {
        return reply_err(&mut send, eio("unerwartetes Schreib-Ende")).await;
    }
    let mutation = req.mutates_filesystem();
    let (access, write_authorization) = match requested_lease {
        Some(token) => {
            match mount_leases.authorize(
                &token,
                &principal,
                &exports,
                legacy_connection,
                node.filesystem_authorization_epoch(),
            ) {
                Ok(lease) => {
                    let authorization = mutation.then(|| {
                        MountLeaseAuthorization::new(
                            token,
                            lease.clone(),
                            session,
                            auth,
                            node,
                            legacy_connection,
                        )
                    });
                    (FsAccess::mounted(lease), authorization)
                }
                Err(error) => return reply_err(&mut send, error).await,
            }
        }
        None => (FsAccess::dynamic(exports), None),
    };
    match req {
        FsRequest::Capabilities { .. } => Err(eio("Capabilities wurden doppelt verarbeitet")),
        FsRequest::ReleaseLease => Err(eio("Lease-Freigabe wurde doppelt verarbeitet")),
        FsRequest::ListDir { path } => {
            match blocking_fs("Share list directory", move || access.list_dir(&path)).await {
                Ok(entries) => reply(&mut send, FsResponse::Entries { entries }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::Stat { path } => {
            match blocking_fs("Share stat", move || access.stat(&path)).await {
                Ok(meta) => reply(&mut send, FsResponse::Meta { meta }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::WalkTree { path } => super::walk::serve_walk(send, path, access).await,
        FsRequest::Read { path } => super::server_transfer::read_file(send, path, access).await,
        FsRequest::Write { path } => {
            super::server_transfer::write_file(
                send,
                recv,
                path,
                access,
                super::server_transfer::WriteMode::Replace,
                write_authorization,
            )
            .await
        }
        FsRequest::WriteNew { path } => {
            super::server_transfer::write_file(
                send,
                recv,
                path,
                access,
                super::server_transfer::WriteMode::Create,
                write_authorization,
            )
            .await
        }
        FsRequest::MkdirAll { path } => {
            simple(
                &mut send,
                path,
                access,
                write_authorization,
                "Share create directory",
                |target| target.backend.mkdir_all(&target.path),
            )
            .await
        }
        FsRequest::Rename { src, dst } => match blocking_fs("Share rename", move || {
            run_authorized(write_authorization.as_ref(), || {
                access.rename(&src, &dst, false)
            })
        })
        .await
        {
            Ok(()) => reply(&mut send, FsResponse::Ok).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::RenameNoReplace { src, dst } => {
            match blocking_fs("Share no-replace rename", move || {
                run_authorized(write_authorization.as_ref(), || {
                    access.rename(&src, &dst, true)
                })
            })
            .await
            {
                Ok(()) => reply(&mut send, FsResponse::Ok).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::PromoteStaged {
            staged,
            destination,
        } => match blocking_fs("Share promote staged file", move || {
            run_authorized(write_authorization.as_ref(), || {
                access.promote_staged(&staged, &destination)
            })
        })
        .await
        {
            Ok(()) => reply(&mut send, FsResponse::Ok).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::CopyFile { src, dst } => {
            let result = blocking_fs("Share copy file", move || {
                run_authorized(write_authorization.as_ref(), || {
                    let source = access.resolve(&src)?;
                    let destination = access.resolve(&dst)?;
                    access.require_same_backend(&source, &destination)?;
                    source.backend.copy_file(&source.path, &destination.path)
                })
            })
            .await;
            match result {
                Ok(size) => reply(&mut send, FsResponse::Data { size }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::RemoveFile { path } => {
            simple(
                &mut send,
                path,
                access,
                write_authorization,
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
                write_authorization,
                "Share remove directory",
                |target| fs::remove_dir_recursive(&*target.backend, &target.path),
            )
            .await
        }
        FsRequest::WriteDone => reply_err(&mut send, eio("unerwartetes Schreib-Ende")).await,
    }
}

async fn handle_capabilities(
    send: &mut SendStream,
    path: String,
    acquire_lease: bool,
    exports: ShareExportConfig,
    principal: PeerPrincipal,
    lease_request_id: Option<String>,
    legacy_connection: usize,
    authorization_epoch: u64,
    mount_leases: Arc<PeerMountLeases>,
) -> io::Result<()> {
    let result = blocking_fs("Share filesystem capabilities", move || {
        if acquire_lease {
            if let Some(grant) = mount_leases.existing_acquisition(
                &path,
                &exports,
                &principal,
                lease_request_id.as_deref(),
                legacy_connection,
                authorization_epoch,
            )? {
                let capabilities = grant.lease.capabilities();
                return Ok(FsResponse::Capabilities {
                    capabilities: capabilities.staged_write.into(),
                    contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                    root_confined: capabilities.root_confinement.is_enforced(),
                    lease: Some(grant.token),
                });
            }
        }
        let snapshot = Arc::new(Mutex::new(exports.clone()));
        let resolved = super::fs_capabilities::resolve_mount_capabilities(&path, &snapshot)?;
        let Some(resolved) = resolved else {
            return Ok(FsResponse::Capabilities {
                capabilities: Default::default(),
                contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                root_confined: false,
                lease: None,
            });
        };
        if !acquire_lease {
            let root_confined = resolved.lease_root_confined();
            return Ok(FsResponse::Capabilities {
                capabilities: resolved.capabilities.staged_write.into(),
                contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
                root_confined,
                lease: None,
            });
        }
        let grant = mount_leases.acquire(
            resolved,
            exports,
            principal,
            lease_request_id,
            legacy_connection,
            authorization_epoch,
        )?;
        let capabilities = grant.lease.capabilities();
        Ok(FsResponse::Capabilities {
            capabilities: capabilities.staged_write.into(),
            contract_version: MOUNT_PATH_CAPABILITY_CONTRACT_VERSION,
            root_confined: capabilities.root_confinement.is_enforced(),
            lease: Some(grant.token),
        })
    })
    .await;
    match result {
        Ok(response) => reply(send, response).await,
        Err(error) => reply_err(send, error).await,
    }
}

async fn handle_exec_stream(
    send: &mut SendStream,
    req: ExecRequest,
    exec_slots: Arc<AtomicUsize>,
) -> io::Result<()> {
    // Filesystem protocol v3 never carries an enabled Exec authorization.
    // A later dedicated Exec ALPN must supply the exact per-device policy.
    let result =
        match super::exec::prepare(req, &super::exec_policy::ExecGrant::default(), exec_slots) {
            Ok(prepared) => tokio::task::spawn_blocking(move || prepared.run())
                .await
                .map_err(|error| eio(format!("remote execution worker failed: {error}")))?,
            Err(error) => Err(error),
        };
    match result {
        Ok(result) => send_ctrl(send, &Ctrl::ExecResp { result }).await,
        Err(error) => {
            send_ctrl(
                send,
                &Ctrl::ExecErr {
                    msg: error.to_string(),
                },
            )
            .await
        }
    }
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
    F: FnOnce(fs::ResolvedTarget) -> io::Result<()> + Send + 'static,
{
    match blocking_fs(label, move || {
        run_authorized(authorization.as_ref(), || {
            access.resolve(&path).and_then(operation)
        })
    })
    .await
    {
        Ok(()) => reply(send, FsResponse::Ok).await,
        Err(error) => reply_err(send, error).await,
    }
}

async fn blocking_fs<T, F>(label: &'static str, operation: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    super::blocking::run(label, operation).await
}
