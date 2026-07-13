use std::io;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::sync::OwnedSemaphorePermit;

use super::connection_events::ConnectionErrorKind;
use super::core::eio;
use super::framing::{recv_ctrl, reply, reply_err, send_ctrl};
use super::fs::{self, ShareExportConfig};
use super::node::ShareIrohNode;
use super::session::{authenticate_incoming_session, IncomingSession};
use super::types::{ExecRequest, ShareEvent};
use super::wire::{Ctrl, FsRequest, FsResponse};

pub(super) async fn handle_connection(
    node: Arc<ShareIrohNode>,
    conn: Connection,
    handshake_permit: OwnedSemaphorePermit,
) -> io::Result<()> {
    let _incoming = node.track_incoming(&conn)?;
    let remote_node = conn.remote_id().to_string();
    let (mut send, mut recv) = tokio::time::timeout(Duration::from_secs(20), conn.accept_bi())
        .await
        .map_err(|_| eio("Session-Handshake Timeout"))?
        .map_err(eio)?;
    let hello = match recv_ctrl(&mut recv).await? {
        Ctrl::PeerHello { hello } => hello,
        _ => return Err(eio("Session-Hello fehlt")),
    };
    if hello.protocol_version != 3 {
        send_ctrl(
            &mut send,
            &Ctrl::FsResp {
                resp: super::fs_error::message("Inkompatibles Share-Protokoll"),
            },
        )
        .await?;
        return Err(eio("Inkompatibles Share-Protokoll"));
    }
    let session = match authenticate_incoming_session(&hello, &remote_node, &node.auth) {
        Ok(session) => session,
        Err(error) => {
            send_ctrl(
                &mut send,
                &Ctrl::FsResp {
                    resp: super::fs_error::response(&error),
                },
            )
            .await?;
            return Err(error);
        }
    };
    send_ctrl(&mut send, &Ctrl::PeerHelloOk).await?;
    drop(handshake_permit);
    let _ = node.ev.send(ShareEvent::Status(format!(
        "Iroh-Session akzeptiert: {} ({})",
        hello.device_id, remote_node
    )));
    let session = Arc::new(session);
    let exec_slots = super::exec::peer_slots(&remote_node);
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
            if let Err(error) = handle_peer_stream(send, recv, session, auth, exec_slots).await {
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
    exec_slots: Arc<AtomicUsize>,
) -> io::Result<()> {
    let ctrl = recv_ctrl(&mut recv).await?;
    let exports = match session.authorize(&auth) {
        Ok(exports) => Arc::new(Mutex::new(exports)),
        Err(error) => {
            return match ctrl {
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
        }
    };
    let req = match ctrl {
        Ctrl::Fs { req } => req,
        Ctrl::Exec { req } => return handle_exec_stream(&mut send, req, exec_slots).await,
        _ => return Err(eio("Dateioperation erwartet")),
    };
    match req {
        FsRequest::ListDir { path } => match blocking_fs("Share list directory", move || {
            fs::list_dir(&path, &exports)
        })
        .await
        {
            Ok(entries) => reply(&mut send, FsResponse::Entries { entries }).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::Stat { path } => {
            match blocking_fs("Share stat", move || fs::stat(&path, &exports)).await {
                Ok(meta) => reply(&mut send, FsResponse::Meta { meta }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::WalkTree { path } => super::walk::serve_walk(send, path, exports).await,
        FsRequest::Read { path } => super::server_transfer::read_file(send, path, exports).await,
        FsRequest::Write { path } => {
            super::server_transfer::write_file(send, recv, path, exports).await
        }
        FsRequest::MkdirAll { path } => {
            simple(
                &mut send,
                path,
                exports,
                "Share create directory",
                |target| target.backend.mkdir_all(&target.path),
            )
            .await
        }
        FsRequest::Rename { src, dst } => match blocking_fs("Share rename", move || {
            fs::rename(&src, &dst, &exports, false)
        })
        .await
        {
            Ok(()) => reply(&mut send, FsResponse::Ok).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::RenameNoReplace { src, dst } => {
            match blocking_fs("Share no-replace rename", move || {
                fs::rename(&src, &dst, &exports, true)
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
            fs::promote_staged(&staged, &destination, &exports)
        })
        .await
        {
            Ok(()) => reply(&mut send, FsResponse::Ok).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::CopyFile { src, dst } => {
            let result = blocking_fs("Share copy file", move || {
                let source = fs::resolve(&src, &exports)?;
                let destination = fs::resolve(&dst, &exports)?;
                if source.mount_key != destination.mount_key {
                    return Err(eio("Quelle und Ziel liegen nicht auf derselben Freigabe"));
                }
                source.backend.copy_file(&source.path, &destination.path)
            })
            .await;
            match result {
                Ok(size) => reply(&mut send, FsResponse::Data { size }).await,
                Err(error) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::RemoveFile { path } => {
            simple(&mut send, path, exports, "Share remove file", |target| {
                target.backend.remove_file(&target.path)
            })
            .await
        }
        FsRequest::RemoveDir { path } => {
            simple(
                &mut send,
                path,
                exports,
                "Share remove directory",
                |target| fs::remove_dir_recursive(&*target.backend, &target.path),
            )
            .await
        }
        FsRequest::WriteDone => reply_err(&mut send, eio("unerwartetes Schreib-Ende")).await,
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
    exports: Arc<Mutex<ShareExportConfig>>,
    label: &'static str,
    operation: F,
) -> io::Result<()>
where
    F: FnOnce(fs::ResolvedTarget) -> io::Result<()> + Send + 'static,
{
    match blocking_fs(label, move || {
        fs::resolve(&path, &exports).and_then(operation)
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
