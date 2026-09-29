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
use super::fs_access::FsAccess;
use super::handshake_limits::ApplicationHandshakePermit;
use super::io_deadline;
use super::mount_lease::MountLeaseAuthorization;
use super::node::ShareIrohNode;
use super::session::{authenticate_incoming_session, IncomingSession};
use super::types::{ExecRequest, ShareAuthState, ShareEvent};
use super::wire::{Ctrl, FsRequest, FsResponse, TRANSFER_V1_CAPABILITY};

#[path = "server_admission.rs"]
mod admission;
#[path = "server_batch_get.rs"]
mod batch_get;
#[path = "server_batch_put.rs"]
mod batch_put;
#[path = "batch_status.rs"]
mod batch_status;
#[path = "server_fs.rs"]
mod fs_dispatch;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

/// What every stream of one authenticated connection shares.
struct StreamContext {
    session: Arc<IncomingSession>,
    auth: Arc<Mutex<ShareAuthState>>,
    node: Arc<ShareIrohNode>,
    exec_slots: Arc<AtomicUsize>,
    legacy_connection: usize,
}

impl StreamContext {
    /// Admits one transfer. A client that declared transfer v1 gets `Busy`
    /// at once when the host is full; older clients keep waiting as before.
    async fn admit(&self) -> io::Result<admission::TransferSlot> {
        let fail_fast =
            self.session.requested(TRANSFER_V1_CAPABILITY) && !self.node.legacy_transfer_host();
        admission::admit(fail_fast).await
    }
}

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
        let context = StreamContext {
            session: session.clone(),
            auth: node.auth.clone(),
            node: node.clone(),
            exec_slots: exec_slots.clone(),
            legacy_connection,
        };
        let node = node.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_peer_stream(send, recv, context).await {
                node.emit_connection_error(ConnectionErrorKind::FsStream, error.to_string());
            }
        });
    }
}

async fn handle_peer_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    context: StreamContext,
) -> io::Result<()> {
    let ctrl = io_deadline::run(
        "Share operation frame",
        recv_ctrl_limited(&mut recv, MAX_REQUEST_CTRL_FRAME),
    )
    .await?;
    let node = context.node.clone();
    let session = context.session.clone();
    let auth = context.auth.clone();
    let legacy_connection = context.legacy_connection;
    node.require_sharing_active()?;
    if matches!(&ctrl, Ctrl::DirectReciprocal) {
        super::direct_reciprocal_transport::serve_incoming_bounded(
            send,
            recv,
            session,
            auth,
            node.direct_repair_store.clone(),
            node.direct_repair_slots.clone(),
            node.runtime_transition_slot.clone(),
            node.ev.clone(),
        )
        .await?;
        return Ok(());
    }
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
        Ctrl::Exec { req } => return handle_exec_stream(&mut send, req, context.exec_slots).await,
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
            let query = super::server_capabilities::CapabilityQuery {
                path,
                acquire_lease,
                lease_request_id,
                exports,
                principal,
                legacy_connection,
                authorization_epoch: node.filesystem_authorization_epoch(),
                mount_leases,
                transfer: node.transfer_capabilities(),
            };
            return super::server_capabilities::handle_capabilities(&mut send, query).await;
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
    // Mutations and every batch entry are admitted again through the lease.
    let admit_each = req.mutates_filesystem() || req.is_batch();
    let (access, authorization) = match requested_lease {
        Some(token) => {
            match mount_leases.authorize(
                &token,
                &principal,
                &exports,
                legacy_connection,
                node.filesystem_authorization_epoch(),
            ) {
                Ok(lease) => {
                    let authorization = admit_each.then(|| {
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
    let stream = fs_dispatch::FsStream {
        send,
        recv,
        access,
        authorization,
        context,
        principal,
    };
    fs_dispatch::serve(stream, req).await
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

async fn blocking_fs<T, F>(label: &'static str, operation: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    super::blocking::run(label, operation).await
}
