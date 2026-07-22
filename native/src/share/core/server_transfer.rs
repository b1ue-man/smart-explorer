use std::io::{self, Read, Write};

use iroh::endpoint::{RecvStream, SendStream};
use tokio::sync::{mpsc, oneshot};

use super::blocking;
use super::core::eio;
use super::framing::{recv_tagged, reply, reply_err, send_tagged, TAG_DATA};
use super::fs;
use super::fs_access::FsAccess;
use super::mount_lease::{run_authorized, MountLeaseAuthorization};
use super::wire::{Ctrl, FsRequest, FsResponse};

const STREAM_BUFFER_CHUNKS: usize = 2;

#[derive(Clone, Copy)]
pub(super) enum WriteMode {
    Replace,
    Create,
}

pub(super) async fn read_file(
    mut send: SendStream,
    path: String,
    access: FsAccess,
) -> io::Result<()> {
    let (ready_tx, ready_rx) = oneshot::channel();
    let (data_tx, mut data_rx) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let worker = blocking::spawn("Share read", move || {
        read_worker(path, access, ready_tx, data_tx)
    })
    .await?;

    let size = match ready_rx.await {
        Ok(Ok(size)) => size,
        Ok(Err(error)) => return reply_err(&mut send, error).await,
        Err(_) => return Err(worker.join().await.unwrap_err_or_worker_exit("Share read")),
    };
    reply(&mut send, FsResponse::Data { size }).await?;
    while let Some(chunk) = data_rx.recv().await {
        send_tagged(&mut send, TAG_DATA, &chunk?).await?;
    }
    worker.join().await
}

fn read_worker(
    path: String,
    access: FsAccess,
    ready: oneshot::Sender<io::Result<u64>>,
    chunks: mpsc::Sender<io::Result<Vec<u8>>>,
) -> io::Result<()> {
    let target = match access.resolve(&path) {
        Ok(target) => target,
        Err(error) => {
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };
    let size = match target.backend.stat(&target.path) {
        Ok(metadata) if !metadata.is_dir => metadata.size,
        Ok(_) => {
            let _ = ready.send(Err(eio("Ordner kann nicht als Datei gelesen werden")));
            return Ok(());
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };
    let mut reader = match target.backend.open_read(&target.path) {
        Ok(reader) => reader,
        Err(error) => {
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };
    if ready.send(Ok(size)).is_err() {
        return Ok(());
    }
    loop {
        let mut buffer = vec![0u8; fs::CHUNK];
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                let _ = chunks.blocking_send(Err(error));
                return Ok(());
            }
        };
        if read == 0 {
            return Ok(());
        }
        buffer.truncate(read);
        if chunks.blocking_send(Ok(buffer)).is_err() {
            return Ok(());
        }
    }
}

pub(super) async fn write_file(
    mut send: SendStream,
    mut recv: RecvStream,
    path: String,
    access: FsAccess,
    mode: WriteMode,
    lease_authorization: Option<MountLeaseAuthorization>,
) -> io::Result<()> {
    let (ready_tx, ready_rx) = oneshot::channel();
    let (command_tx, command_rx) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let (done_tx, done_rx) = oneshot::channel();
    let expected_lease = lease_authorization
        .as_ref()
        .map(|authorization| authorization.token().to_string());
    let worker = blocking::spawn("Share staged write", move || {
        write_worker(
            path,
            access,
            mode,
            lease_authorization,
            ready_tx,
            command_rx,
            done_tx,
        )
    })
    .await?;

    match ready_rx.await {
        Ok(Ok(())) => reply(&mut send, FsResponse::Ready).await?,
        Ok(Err(error)) => return reply_err(&mut send, error).await,
        Err(_) => {
            return Err(worker
                .join()
                .await
                .unwrap_err_or_worker_exit("Share staged write"))
        }
    }

    let mut done_rx = Some(done_rx);
    let mut worker = Some(worker);
    loop {
        let (tag, payload) = match recv_tagged(&mut recv).await {
            Ok(frame) => frame,
            Err(error) => {
                drop(command_tx);
                await_write_cleanup(&mut done_rx, &mut worker).await;
                return Err(error);
            }
        };
        if tag == TAG_DATA {
            if command_tx.send(WriteCommand::Data(payload)).await.is_err() {
                let error = await_write_error(&mut done_rx, &mut worker).await;
                return reply_err(&mut send, error).await;
            }
            continue;
        }
        if tag != super::framing::TAG_CTRL {
            drop(command_tx);
            await_write_cleanup(&mut done_rx, &mut worker).await;
            return reply_err(&mut send, eio("unerwarteter Frame beim Schreiben")).await;
        }
        match serde_json::from_slice::<Ctrl>(&payload).map_err(eio) {
            Ok(Ctrl::Fs {
                req: FsRequest::WriteDone,
                lease,
            }) => {
                if let Some(expected) = expected_lease.as_deref() {
                    if lease.as_deref() != Some(expected) {
                        drop(command_tx);
                        await_write_cleanup(&mut done_rx, &mut worker).await;
                        return reply_err(
                            &mut send,
                            io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "Peer-Mount-Lease fehlt beim Schreibabschluss",
                            ),
                        )
                        .await;
                    }
                }
                if command_tx.send(WriteCommand::Finish).await.is_err() {
                    let error = await_write_error(&mut done_rx, &mut worker).await;
                    return reply_err(&mut send, error).await;
                }
                drop(command_tx);
                return match await_write_result(&mut done_rx, &mut worker).await {
                    Ok(()) => reply(&mut send, FsResponse::Ok).await,
                    Err(error) => reply_err(&mut send, error).await,
                };
            }
            Ok(_) => {
                drop(command_tx);
                await_write_cleanup(&mut done_rx, &mut worker).await;
                return reply_err(&mut send, eio("unerwartete Steuernachricht beim Schreiben"))
                    .await;
            }
            Err(error) => {
                drop(command_tx);
                await_write_cleanup(&mut done_rx, &mut worker).await;
                return reply_err(&mut send, error).await;
            }
        }
    }
}

enum WriteCommand {
    Data(Vec<u8>),
    Finish,
}

fn write_worker(
    path: String,
    access: FsAccess,
    mode: WriteMode,
    lease_authorization: Option<MountLeaseAuthorization>,
    ready: oneshot::Sender<io::Result<()>>,
    mut commands: mpsc::Receiver<WriteCommand>,
    done: oneshot::Sender<io::Result<()>>,
) -> io::Result<()> {
    // Opening the private stage/new target is the first mutation admission.
    let prepared = run_authorized(lease_authorization.as_ref(), || {
        let target = access.resolve(&path)?;
        let opened = match mode {
            WriteMode::Replace => {
                crate::vfs::unique_staging_path(&*target.backend, &target.path, "peer").and_then(
                    |staging| {
                        target
                            .backend
                            .open_write_new(&staging)
                            .map(|writer| (staging, writer, true))
                    },
                )
            }
            WriteMode::Create => target
                .backend
                .open_write_new(&target.path)
                .map(|writer| (target.path.clone(), writer, false)),
        }?;
        Ok((target, opened))
    });
    let (target, (staging, writer, replace_after_upload)) = match prepared {
        Ok(opened) => opened,
        Err(error) => {
            // An exclusive-open failure never transfers ownership of this
            // spelling; deleting it could remove a concurrent case alias.
            let _ = ready.send(Err(error));
            return Ok(());
        }
    };
    if ready.send(Ok(())).is_err() {
        drop(writer);
        // This layer has no stable identity for the exclusively created item.
        // A concurrent actor may have moved it and reused the spelling.
        return Ok(());
    }

    let mut writer = Some(writer);
    let result = loop {
        match commands.blocking_recv() {
            Some(WriteCommand::Data(payload)) => {
                let Some(output) = writer.as_mut() else {
                    break Err(eio("Schreibkanal ist geschlossen"));
                };
                if let Err(error) = output.write_all(&payload) {
                    break Err(error);
                }
            }
            Some(WriteCommand::Finish) => {
                // A multi-phase write is admitted again at its commit boundary:
                // a revoke since writer creation prevents flush/promotion.
                break run_authorized(lease_authorization.as_ref(), || {
                    let Some(mut output) = writer.take() else {
                        return Err(eio("Schreibkanal ist geschlossen"));
                    };
                    output.flush()?;
                    drop(output);
                    if replace_after_upload {
                        crate::vfs::promote_staged_replace(&*target.backend, &staging, &target.path)
                    } else {
                        Ok(())
                    }
                });
            }
            None => {
                break Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "peer write canceled",
                ))
            }
        }
    };
    if result.is_err() {
        drop(writer.take());
        // Flush and promotion failures can be lost-ack outcomes. Retain the
        // stage rather than risk unlinking a replacement by path.
    }
    let _ = done.send(result);
    Ok(())
}

async fn await_write_result(
    done: &mut Option<oneshot::Receiver<io::Result<()>>>,
    worker: &mut Option<blocking::BlockingTask<()>>,
) -> io::Result<()> {
    let result = match done.take() {
        Some(done) => done
            .await
            .unwrap_or_else(|_| Err(eio("Share write worker exited"))),
        None => Err(eio("Share write result is missing")),
    };
    if let Some(worker) = worker.take() {
        worker.join().await?;
    }
    result
}

async fn await_write_cleanup(
    done: &mut Option<oneshot::Receiver<io::Result<()>>>,
    worker: &mut Option<blocking::BlockingTask<()>>,
) {
    let _ = await_write_result(done, worker).await;
}

async fn await_write_error(
    done: &mut Option<oneshot::Receiver<io::Result<()>>>,
    worker: &mut Option<blocking::BlockingTask<()>>,
) -> io::Error {
    match await_write_result(done, worker).await {
        Ok(()) => eio("Share write worker stopped before accepting the command"),
        Err(error) => error,
    }
}

trait WorkerExit<T> {
    fn unwrap_err_or_worker_exit(self, operation: &str) -> io::Error;
}

impl<T> WorkerExit<T> for io::Result<T> {
    fn unwrap_err_or_worker_exit(self, operation: &str) -> io::Error {
        match self {
            Ok(_) => eio(format!("{operation} worker exited without a result")),
            Err(error) => error,
        }
    }
}
