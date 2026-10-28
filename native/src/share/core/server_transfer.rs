use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream};
use tokio::sync::{mpsc, oneshot};

use super::blocking;
use super::core::eio;
use super::framing::lz4_stream::{self, classify, Content};
use super::framing::{recv_content_frame, reply, reply_err};
use super::fs_access::FsAccess;
use super::io_deadline;
use super::mount_lease::{run_authorized, MountLeaseAuthorization};
use super::transfer_meter as stats;
use super::wire::{Ctrl, FsRequest, FsResponse, FsWritten};

#[path = "server_read.rs"]
mod server_read;
pub(super) use server_read::{read_file, stat_item, ReadSource};

/// Chunks a transfer worker may run ahead of its stream (512 KiB): one disk
/// read overlaps one network send; admission reserves this memory per worker.
pub(super) const STREAM_BUFFER_CHUNKS: usize = 2;

#[derive(Clone, Copy)]
pub(super) enum WriteMode {
    Replace,
    Create,
}

/// What a Write or WriteNew names and the lease its commit repeats.
pub(super) struct WriteTarget {
    pub(super) path: String,
    pub(super) mode: WriteMode,
    pub(super) authorization: Option<MountLeaseAuthorization>,
    /// The client may send LZ4 blocks (`lz4_v1`).
    pub(super) lz4: bool,
    /// `evidence_v1` WriteNew: the commit answers `Written`, naming whether
    /// the inner path (a sibling of the new file) existed.
    pub(super) evidence: Option<Option<String>>,
}

/// The `slot` (transfer admission) stays held until the worker ends; a
/// client that sends nothing for `stall` loses it.
pub(super) async fn write_file<G: Send + 'static>(
    mut send: SendStream,
    mut recv: RecvStream,
    target: WriteTarget,
    access: FsAccess,
    slot: G,
    stall: Duration,
) -> io::Result<()> {
    let expected_lease = target
        .authorization
        .as_ref()
        .map(|authorization| authorization.token().to_string());
    let lz4 = lz4_stream::negotiated(target.lz4);
    let (ready_tx, ready_rx) = oneshot::channel();
    let (command_tx, command_rx) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let (done_tx, done_rx) = oneshot::channel();
    let abandoned = Arc::new(AtomicBool::new(false));
    let job = WriteJob {
        target,
        access,
        abandoned: abandoned.clone(),
    };
    let worker = blocking::spawn_holding("Share staged write", slot, move || {
        write_worker(job, ready_tx, command_rx, done_tx)
    });

    let stopped = send.stopped();
    let ready = tokio::select! {
        ready = ready_rx => ready,
        _ = stopped => {
            // The client gave up before its target was open: it must not be
            // created afterwards (a WriteNew target would stay behind).
            abandoned.store(true, Ordering::Release);
            drop(command_tx);
            await_write_cleanup(&mut Some(done_rx), &mut Some(worker)).await;
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Client hat das Schreiben vor dem Öffnen aufgegeben",
            ));
        }
    };
    match ready {
        Ok(Ok(())) => reply(&mut send, FsResponse::Ready).await?,
        Ok(Err(error)) => return reply_err(&mut send, error).await,
        Err(_) => {
            return Err(worker
                .join()
                .await
                .unwrap_err_or_worker_exit("Share staged write"))
        }
    }
    stats::stream_started();

    let mut done_rx = Some(done_rx);
    let mut worker = Some(worker);
    loop {
        let frame = io_deadline::run_for(
            "Share write data",
            stall,
            recv_content_frame(&mut recv, lz4),
        );
        // Waiting for the next frame is waiting for the network.
        let waiting = stats::Timer::start();
        let frame = frame.await;
        waiting.stop(stats::Phase::NetWait);
        let (tag, payload) = match frame {
            Ok(frame) => frame,
            Err(error) => {
                drop(command_tx);
                await_write_cleanup(&mut done_rx, &mut worker).await;
                return Err(error);
            }
        };
        let control = match classify(tag, payload, lz4) {
            Ok(Content::Bytes(bytes)) => {
                // A full queue is the writer being slower than the network.
                let queued = stats::Timer::start();
                let sent = command_tx.send(WriteCommand::Data(bytes)).await;
                queued.stop(stats::Phase::StageWait);
                if sent.is_err() {
                    let error = await_write_error(&mut done_rx, &mut worker).await;
                    return reply_err(&mut send, error).await;
                }
                continue;
            }
            Ok(Content::Other(control)) => control,
            Err(error) => {
                drop(command_tx);
                await_write_cleanup(&mut done_rx, &mut worker).await;
                return reply_err(&mut send, error).await;
            }
        };
        if tag != super::framing::TAG_CTRL {
            drop(command_tx);
            await_write_cleanup(&mut done_rx, &mut worker).await;
            return reply_err(&mut send, eio("unerwarteter Frame beim Schreiben")).await;
        }
        match serde_json::from_slice::<Ctrl>(&control).map_err(eio) {
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
                    Ok(None) => reply(&mut send, FsResponse::Ok).await,
                    Ok(Some(written)) => reply(&mut send, FsResponse::Written { written }).await,
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

/// What the write worker opens, and whether its client already left.
struct WriteJob {
    target: WriteTarget,
    access: FsAccess,
    abandoned: Arc<AtomicBool>,
}

/// What a committed write reports: `Written` for an `evidence_v1` WriteNew.
type WriteDone = io::Result<Option<FsWritten>>;

fn write_worker(
    job: WriteJob,
    ready: oneshot::Sender<io::Result<()>>,
    mut commands: mpsc::Receiver<WriteCommand>,
    done: oneshot::Sender<WriteDone>,
) -> io::Result<()> {
    let WriteJob {
        target:
            WriteTarget {
                path,
                mode,
                authorization: lease_authorization,
                // Unpacking the blocks happens before the worker (`write_file`).
                lz4: _,
                evidence,
            },
        access,
        abandoned,
    } = job;
    if abandoned.load(Ordering::Acquire) {
        let _ = ready.send(Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "Client hat das Schreiben vor dem Öffnen aufgegeben",
        )));
        return Ok(());
    }
    // Opening the private stage/new target is the first mutation admission.
    let prepared = run_authorized(lease_authorization.as_ref(), || {
        let target = access.resolve_write(&path)?;
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
                let writing = stats::Timer::start();
                let written = output.write_all(&payload);
                writing.stop(stats::Phase::StageWrite);
                if let Err(error) = written {
                    break Err(error);
                }
            }
            Some(WriteCommand::Finish) => {
                // A multi-phase write is admitted again at its commit boundary:
                // a revoke since writer creation prevents flush/promotion.
                break run_authorized(lease_authorization.as_ref(), || {
                    access.resolve_write(&path)?;
                    let Some(mut output) = writer.take() else {
                        return Err(eio("Schreibkanal ist geschlossen"));
                    };
                    output.flush()?;
                    drop(output);
                    if replace_after_upload {
                        crate::vfs::promote_staged_replace(&*target.backend, &staging, &target.path)
                            .map(|()| None)
                    } else {
                        let probe = |probe: &Option<String>| {
                            written(&access, &path, &target, probe.as_deref())
                        };
                        Ok(evidence.as_ref().and_then(probe))
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

/// The evidence of the closed new file `path` (`target`) and whether the
/// sibling `probe` exists; `None` when it cannot be read (the client then
/// asks for it as before).
fn written(
    access: &FsAccess,
    path: &str,
    target: &super::fs::ResolvedTarget,
    probe: Option<&str>,
) -> Option<FsWritten> {
    let mut scopes = super::host_evidence::Scopes::default();
    let evidence = super::host_evidence::of_target(access, path, target, &mut scopes)?;
    let existed = probe
        .filter(|probe| sibling(path, probe))
        .and_then(|probe| access.resolve(probe).ok())
        .and_then(|probe| super::host_evidence::exists(&probe));
    Some(FsWritten { evidence, existed })
}

/// Whether `probe` lies in the same peer folder as `path`.
fn sibling(path: &str, probe: &str) -> bool {
    let parent = |path: &str| path.rsplit_once('/').map(|(parent, _)| parent.to_string());
    parent(path).is_some() && parent(path) == parent(probe)
}

async fn await_write_result(
    done: &mut Option<oneshot::Receiver<WriteDone>>,
    worker: &mut Option<blocking::BlockingTask<()>>,
) -> WriteDone {
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
    done: &mut Option<oneshot::Receiver<WriteDone>>,
    worker: &mut Option<blocking::BlockingTask<()>>,
) {
    let _ = await_write_result(done, worker).await;
}

async fn await_write_error(
    done: &mut Option<oneshot::Receiver<WriteDone>>,
    worker: &mut Option<blocking::BlockingTask<()>>,
) -> io::Error {
    match await_write_result(done, worker).await {
        Ok(_) => eio("Share write worker stopped before accepting the command"),
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

#[cfg(test)]
#[path = "server_transfer_tests.rs"]
mod tests;
