use std::io::{self, Read, Write};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};

use super::core::eio;
use super::framing::{recv_ctrl, recv_tagged, reply, reply_err, send_ctrl, send_tagged, TAG_DATA};
use super::fs::{self, ShareExportConfig};
use super::node::ShareIrohNode;
use super::session::{authenticate_incoming_session, IncomingSession};
use super::types::{ExecRequest, ShareEvent};
use super::wire::{Ctrl, FsRequest, FsResponse};

pub(super) async fn handle_connection(
    node: Arc<ShareIrohNode>,
    conn: Connection,
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
        let events = node.ev.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_peer_stream(send, recv, session, auth, exec_slots).await {
                let _ = events.send(ShareEvent::Error(format!("Iroh-FS: {error}")));
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
        Ctrl::Exec { req } => {
            return handle_exec_stream(&mut send, req, &exports, exec_slots).await
        }
        _ => return Err(eio("Dateioperation erwartet")),
    };
    match req {
        FsRequest::ListDir { path } => match fs::list_dir(&path, &exports) {
            Ok(entries) => reply(&mut send, FsResponse::Entries { entries }).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::Stat { path } => match fs::stat(&path, &exports) {
            Ok(meta) => reply(&mut send, FsResponse::Meta { meta }).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::WalkTree { path } => super::walk::serve_walk(&mut send, path, exports).await,
        FsRequest::Read { path } => read_file(&mut send, &path, &exports).await,
        FsRequest::Write { path } => write_file(&mut send, &mut recv, &path, &exports).await,
        FsRequest::MkdirAll { path } => {
            simple(&mut send, &path, &exports, |target| {
                target.backend.mkdir_all(&target.path)
            })
            .await
        }
        FsRequest::Rename { src, dst } => match fs::rename(&src, &dst, &exports, false) {
            Ok(()) => reply(&mut send, FsResponse::Ok).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::RenameNoReplace { src, dst } => match fs::rename(&src, &dst, &exports, true) {
            Ok(()) => reply(&mut send, FsResponse::Ok).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::PromoteStaged {
            staged,
            destination,
        } => match fs::promote_staged(&staged, &destination, &exports) {
            Ok(()) => reply(&mut send, FsResponse::Ok).await,
            Err(error) => reply_err(&mut send, error).await,
        },
        FsRequest::CopyFile { src, dst } => {
            match (fs::resolve(&src, &exports), fs::resolve(&dst, &exports)) {
                (Ok(source), Ok(destination)) if source.mount_key == destination.mount_key => {
                    match source.backend.copy_file(&source.path, &destination.path) {
                        Ok(size) => reply(&mut send, FsResponse::Data { size }).await,
                        Err(error) => reply_err(&mut send, error).await,
                    }
                }
                (Ok(_), Ok(_)) => {
                    reply_err(
                        &mut send,
                        eio("Quelle und Ziel liegen nicht auf derselben Freigabe"),
                    )
                    .await
                }
                (Err(error), _) | (_, Err(error)) => reply_err(&mut send, error).await,
            }
        }
        FsRequest::RemoveFile { path } => {
            simple(&mut send, &path, &exports, |target| {
                target.backend.remove_file(&target.path)
            })
            .await
        }
        FsRequest::RemoveDir { path } => {
            simple(&mut send, &path, &exports, |target| {
                fs::remove_dir_recursive(&*target.backend, &target.path)
            })
            .await
        }
        FsRequest::WriteDone => reply_err(&mut send, eio("unerwartetes Schreib-Ende")).await,
    }
}

async fn handle_exec_stream(
    send: &mut SendStream,
    req: ExecRequest,
    exports: &Arc<Mutex<ShareExportConfig>>,
    exec_slots: Arc<AtomicUsize>,
) -> io::Result<()> {
    let config = exports
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let result = match super::exec::prepare(req, &config, exec_slots) {
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
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
    operation: F,
) -> io::Result<()>
where
    F: FnOnce(fs::ResolvedTarget) -> io::Result<()>,
{
    match fs::resolve(path, exports).and_then(operation) {
        Ok(()) => reply(send, FsResponse::Ok).await,
        Err(error) => reply_err(send, error).await,
    }
}

async fn read_file(
    send: &mut SendStream,
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    let target = match fs::resolve(path, exports) {
        Ok(target) => target,
        Err(error) => return reply_err(send, error).await,
    };
    let size = match target.backend.stat(&target.path) {
        Ok(metadata) if !metadata.is_dir => metadata.size,
        Ok(_) => return reply_err(send, eio("Ordner kann nicht als Datei gelesen werden")).await,
        Err(error) => return reply_err(send, error).await,
    };
    let mut reader = match target.backend.open_read(&target.path) {
        Ok(reader) => reader,
        Err(error) => return reply_err(send, error).await,
    };
    reply(send, FsResponse::Data { size }).await?;
    let mut buffer = vec![0u8; fs::CHUNK];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        send_tagged(send, TAG_DATA, &buffer[..read]).await?;
    }
    Ok(())
}

async fn write_file(
    send: &mut SendStream,
    recv: &mut RecvStream,
    path: &str,
    exports: &Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    let target = match fs::resolve(path, exports) {
        Ok(target) => target,
        Err(error) => return reply_err(send, error).await,
    };
    let staging = match crate::vfs::unique_staging_path(&*target.backend, &target.path, "peer") {
        Ok(path) => path,
        Err(error) => return reply_err(send, error).await,
    };
    let mut writer = match target.backend.open_write(&staging) {
        Ok(writer) => Some(writer),
        Err(error) => return reply_err(send, error).await,
    };
    reply(send, FsResponse::Ready).await?;
    let result = loop {
        let (tag, payload) = match recv_tagged(recv).await {
            Ok(frame) => frame,
            Err(error) => break Err(error),
        };
        if tag == TAG_DATA {
            let Some(output) = writer.as_mut() else {
                break Err(eio("Schreibkanal ist geschlossen"));
            };
            if let Err(error) = output.write_all(&payload) {
                break Err(error);
            }
            continue;
        }
        if tag != super::framing::TAG_CTRL {
            break Err(eio("unerwarteter Frame beim Schreiben"));
        }
        match serde_json::from_slice::<Ctrl>(&payload).map_err(eio) {
            Ok(Ctrl::Fs {
                req: FsRequest::WriteDone,
            }) => {
                let Some(mut output) = writer.take() else {
                    break Err(eio("Schreibkanal ist geschlossen"));
                };
                if let Err(error) = output.flush() {
                    break Err(error);
                }
                drop(output);
                break crate::vfs::promote_staged_replace(&*target.backend, &staging, &target.path);
            }
            Ok(_) => break Err(eio("unerwartete Steuernachricht beim Schreiben")),
            Err(error) => break Err(error),
        }
    };
    match result {
        Ok(()) => reply(send, FsResponse::Ok).await,
        Err(error) => {
            let message = error.to_string();
            drop(writer.take());
            let _ = target.backend.remove_file(&staging);
            reply_err(send, eio(message)).await
        }
    }
}
