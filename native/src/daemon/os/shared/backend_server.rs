use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::agent::vfs_to_wire;
use crate::agent_proto::{self, busy_message, Frame, RequestContext, ServerSession, PROTO_VERSION};
use crate::vfs::BackendHandle;

use super::backend_batch::{handle_get_batch_backend, handle_put_batch_backend};
use super::backend_delete::remove_tree_backend;
use super::backend_stream::{handle_read_backend, handle_write_backend, WriteMode};
use super::backend_transfer::handle_put_tree_backend;
use super::backend_tree_send::handle_get_tree_backend;
use super::backend_walk::{
    handle_search_backend, handle_walk_hashed_backend, handle_walk_tree_backend, remove_one_backend,
};
use super::request_workers::{RequestWorkers, LEGACY_MAX_REQUEST_WORKERS};

#[path = "backend_ranges.rs"]
mod backend_ranges;
use backend_ranges::BackendStages;

pub(super) type Sink = Arc<Mutex<Box<dyn Write + Send>>>;

pub(super) fn emit(sink: &Sink, id: u64, frame: &Frame) -> io::Result<()> {
    let mut w = sink
        .lock()
        .map_err(|_| io::Error::other("daemon backend writer locked"))?;
    agent_proto::write_frame(&mut *w, id, frame)
}

/// Error text for the client; a credit client reads congestion of the peer
/// (rate limit, busy host) as congestion instead of as a failure.
pub(super) fn error_text(error: &io::Error, credit: bool) -> String {
    match crate::vfs::congestion_of(error) {
        Some(congestion) if credit => busy_message(congestion.retry_after, &congestion.message),
        _ => crate::vault::wire_error_text(error)
            .unwrap_or_else(|| crate::agent_proto::transport_error_message(error)),
    }
}

fn canceled_request_lost_client(
    request_error: &io::Error,
    report_error: &io::Error,
    canceled: bool,
) -> bool {
    canceled
        && matches!(
            request_error.kind(),
            io::ErrorKind::Interrupted | io::ErrorKind::UnexpectedEof
        )
        && matches!(
            report_error.kind(),
            io::ErrorKind::BrokenPipe
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::NotConnected
                | io::ErrorKind::UnexpectedEof
        )
}

/// The version this service announces for `backend`: batches only when the
/// peer moves them itself (never emulated file by file), and no more
/// transfer slots than the peer admits. A Share peer's root is its export
/// list, which holds no files, so its batches are asked for host-wide
/// (Z-P1); each batch request is still checked against its own folder.
fn service_version(backend: &BackendHandle) -> String {
    let root = backend.root_display();
    let batch = crate::vfs::host_batch_limits(&**backend).is_some();
    let slots = agent_proto::service_slots(backend.transfer_ceiling(&root));
    // `range-v1` and `lz4-v1` only for a backend that reads ranges: the
    // frames behind them are forwarded to it (`backend_ranges`).
    format!(
        "{}{}.capacity-v1.folder-reservation-v1.file-identity-v1 worker",
        agent_proto::server_version_with(batch, slots),
        backend_ranges::labels(backend)
    )
}

/// The frames of an upload with `DataLz4` unpacked, for a backend that was
/// announced with `lz4-v1`; for any other backend they are passed unchanged.
fn lz4_inbound<'a>(
    backend: &BackendHandle,
    inbound: &'a (dyn agent_proto::Inbound + Send),
) -> Box<dyn agent_proto::Inbound + 'a> {
    if backend_ranges::offers_ranges(backend) {
        Box::new(agent_proto::Lz4Inbound(inbound))
    } else {
        Box::new(Passthrough(inbound))
    }
}

struct Passthrough<'a>(&'a (dyn agent_proto::Inbound + Send));

impl agent_proto::Inbound for Passthrough<'_> {
    fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Frame, std::sync::mpsc::RecvTimeoutError> {
        self.0.recv_timeout(timeout)
    }
}

/// The legacy text of the busy reply when no thread can be started; "too
/// many concurrent" is what older clients classify as overload.
const THREAD_LIMIT_TEXT: &str = "too many concurrent backend requests (thread limit)";

/// How a request whose worker thread could not start is answered.
#[derive(Debug, PartialEq, Eq)]
enum SpawnFailureReply {
    /// "Busy" (overload), the answer to a refused thread (`EAGAIN`).
    Busy,
    Failed(String),
}

/// A refused thread (`WouldBlock`, `EAGAIN`) is overload; every other reason
/// stays an error of the request.
fn spawn_failure_reply(error: &io::Error) -> SpawnFailureReply {
    if error.kind() == io::ErrorKind::WouldBlock {
        SpawnFailureReply::Busy
    } else {
        SpawnFailureReply::Failed(format!("backend worker could not start: {error}"))
    }
}

pub(crate) fn serve_backend(
    mut r: impl Read,
    w: impl Write + Send + 'static,
    backend: BackendHandle,
) -> io::Result<()> {
    let session = ServerSession::new(Arc::new(Mutex::new(Box::new(w))));
    let version = Arc::new(OnceLock::new());
    // Stages of ranged uploads: owned by this session, aborted with it.
    let stages = BackendStages::new();
    let mut workers = RequestWorkers::default();

    loop {
        let next = match agent_proto::read_frame(&mut r) {
            Ok(next) => next,
            Err(error) => {
                session.abort_all();
                return match workers.shutdown() {
                    Ok(()) => Err(error),
                    Err(shutdown) => Err(io::Error::new(
                        error.kind(),
                        format!(
                            "backend input failed ({error}); worker shutdown failed: {shutdown}"
                        ),
                    )),
                };
            }
        };
        let Some((id, frame)) = next else {
            break;
        };
        let Some(req) = session.route(id, frame) else {
            continue;
        };
        let limit = session.request_limit(LEGACY_MAX_REQUEST_WORKERS);
        let has_capacity = match workers.has_capacity(limit) {
            Ok(has_capacity) => has_capacity,
            Err(worker_error) => {
                session.abort_all();
                return match workers.shutdown() {
                    Ok(()) => Err(worker_error),
                    Err(shutdown_error) => Err(io::Error::other(format!(
                        "backend request worker failed ({worker_error}); worker shutdown failed: {shutdown_error}"
                    ))),
                };
            }
        };
        if !has_capacity {
            session.reject_busy(id, "too many concurrent backend requests")?;
            continue;
        }
        if session.is_active(id) {
            emit(
                session.sink(),
                id,
                &Frame::Err("duplicate active request id".into()),
            )?;
            continue;
        }
        let context = session.open(id, &req);
        let credit = session.credit_mode();
        let worker_session = session.clone();
        let backend2 = backend.clone();
        let version2 = version.clone();
        let stages2 = stages.clone();
        match std::thread::Builder::new()
            .name(format!("daemon-backend-request-{id}"))
            .spawn(move || {
                let result =
                    dispatch_backend(&context, id, (backend2, &stages2), req, &version2, credit);
                let result = match result {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        match emit(&context.sink, id, &Frame::Err(error_text(&error, credit))) {
                            // The request failure has been surfaced to the
                            // client; only a reporting failure remains a
                            // worker-level error that tears down the link.
                            Ok(()) => Ok(()),
                            Err(report_error)
                                if canceled_request_lost_client(
                                    &error,
                                    &report_error,
                                    context.cancel.load(Ordering::Relaxed),
                                ) =>
                            {
                                Ok(())
                            }
                            Err(report_error) => Err(io::Error::new(
                                error.kind(),
                                format!(
                                    "request failed ({error}); reporting it failed: {report_error}"
                                ),
                            )),
                        }
                    }
                };
                worker_session.close(id);
                result
            }) {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                session.discard(id);
                match spawn_failure_reply(&error) {
                    // A process at its thread limit is overloaded, like one
                    // with too many requests: the client waits and retries.
                    SpawnFailureReply::Busy => session.reject_busy(id, THREAD_LIMIT_TEXT)?,
                    SpawnFailureReply::Failed(text) => emit(session.sink(), id, &Frame::Err(text))?,
                }
            }
        }
    }
    session.abort_all();
    let result = workers.shutdown();
    stages.close();
    result
}

fn reply(sink: &Sink, id: u64, result: io::Result<Frame>, credit: bool) -> io::Result<()> {
    match result {
        Ok(frame) => emit(sink, id, &frame),
        Err(error) => emit(sink, id, &Frame::Err(error_text(&error, credit))),
    }
}

fn dispatch_backend(
    context: &RequestContext,
    id: u64,
    (backend, stages): (BackendHandle, &Arc<BackendStages>),
    req: Frame,
    version: &OnceLock<String>,
    credit: bool,
) -> io::Result<()> {
    let sink = &context.sink;
    let cancel: &AtomicBool = &context.cancel;
    let inbound = context.inbound.as_deref();
    let answer = |result: io::Result<Frame>| reply(sink, id, result, credit);
    match req {
        Frame::Hello { .. } => emit(
            sink,
            id,
            &Frame::HelloOk {
                proto: PROTO_VERSION,
                version: version.get_or_init(|| service_version(&backend)).clone(),
            },
        ),
        Frame::ListDir(p) => answer(
            backend
                .list_dir(&p)
                .map(|v| Frame::Dir(v.into_iter().map(vfs_to_wire).collect())),
        ),
        Frame::Stat(p) => answer(backend.stat(&p).map(|m| Frame::Meta(vfs_to_wire(m)))),
        Frame::TryExists(p) => answer(backend.try_exists(&p).map(Frame::Exists)),
        Frame::WalkTree(root) => handle_walk_tree_backend(sink, id, &backend, &root, cancel),
        Frame::Read { path, offset, len } => {
            handle_read_backend(sink, id, &backend, &path, offset, len, cancel)
        }
        Frame::Write(path) => match inbound {
            Some(rx) => handle_write_backend(
                sink,
                id,
                &backend,
                &path,
                &*lz4_inbound(&backend, rx),
                cancel,
                WriteMode::Replace,
            ),
            None => emit(sink, id, &Frame::Err("write: no inbound channel".into())),
        },
        Frame::WriteNew(path) => match inbound {
            Some(rx) => handle_write_backend(
                sink,
                id,
                &backend,
                &path,
                &*lz4_inbound(&backend, rx),
                cancel,
                WriteMode::Create,
            ),
            None => emit(
                sink,
                id,
                &Frame::Err("write-new: no inbound channel".into()),
            ),
        },
        Frame::Copy { src, dst } => answer(backend.copy_file(&src, &dst).map(|_| Frame::Ok)),
        Frame::Rename { src, dst } => answer(backend.rename(&src, &dst).map(|()| Frame::Ok)),
        Frame::RenameNoReplace { src, dst } => {
            answer(backend.rename_no_replace(&src, &dst).map(|()| Frame::Ok))
        }
        Frame::Promote {
            staged,
            destination,
        } => answer(
            backend
                .promote_staged(&staged, &destination)
                .map(|()| Frame::Ok),
        ),
        Frame::PromoteNoReplace {
            staged,
            destination,
        } => answer(
            backend
                .promote_staged_no_replace(&staged, &destination)
                .map(|()| Frame::Ok),
        ),
        Frame::Remove { path, recursive } => {
            let res = if recursive {
                remove_tree_backend(&backend, &path, cancel)
            } else {
                remove_one_backend(&backend, &path)
            };
            answer(res.map(|_| Frame::Ok))
        }
        Frame::Mkdir(p) => answer(backend.mkdir_all(&p).map(|()| Frame::Ok)),
        Frame::GetTree(root) => handle_get_tree_backend(sink, id, &backend, &root, cancel),
        Frame::PutTree(root) => match inbound {
            Some(rx) => handle_put_tree_backend(sink, id, &backend, &root, rx, cancel),
            None => emit(sink, id, &Frame::Err("put-tree: no inbound channel".into())),
        },
        Frame::Search { root, spec } => {
            handle_search_backend(sink, id, &backend, &root, &spec, cancel)
        }
        Frame::WalkHashed { root, want_hash } => {
            handle_walk_hashed_backend(sink, id, &backend, &root, want_hash, cancel)
        }
        Frame::BatchPut { entries } => match inbound {
            Some(rx) => handle_put_batch_backend(sink, id, &backend, &entries, rx, cancel, credit),
            None => emit(sink, id, &Frame::Err("batch: no inbound channel".into())),
        },
        Frame::BatchGet { items } => {
            handle_get_batch_backend(sink, id, &backend, &items, cancel, credit)
        }
        Frame::CopyToStage { src, stage, size } => answer(
            backend
                .server_copy_to_stage(&src, &stage, size, cancel)
                .map(Frame::Copied),
        ),
        Frame::CreateDir { path, exclusive } => answer(
            if exclusive {
                backend.create_dir_new(&path)
            } else {
                backend.create_dir(&path)
            }
            .map(|()| Frame::Ok),
        ),
        Frame::DiscardStage(stage) => {
            if stages.discard(&stage) {
                emit(sink, id, &Frame::Ok)
            } else {
                answer(backend.discard_copy_stage(&stage).map(|()| Frame::Ok))
            }
        }
        // The frames of `range-v1` and `lz4-v1` are served only for a
        // backend that was announced with the labels.
        Frame::ReadLz4 { path, offset, len } if backend_ranges::offers_ranges(&backend) => {
            backend_ranges::read_lz4(sink, id, &backend, &path, (offset, len), cancel)
        }
        Frame::SegStageOpen { path, size } if backend_ranges::offers_ranges(&backend) => {
            // An empty name tells the client that this backend has no stage
            // for ranges: it sends one stream.
            answer(
                stages
                    .open(&backend, &path, size)
                    .map(|stage| Frame::SegStageReady {
                        stage: stage.unwrap_or_default(),
                    }),
            )
        }
        Frame::WriteAt { stage, offset, len } if backend_ranges::offers_ranges(&backend) => {
            match inbound {
                Some(rx) => {
                    backend_ranges::write_at(sink, id, stages, (&stage, offset, len), rx, cancel)
                }
                None => emit(sink, id, &Frame::Err("write-at: no inbound channel".into())),
            }
        }
        Frame::SegStageFinish { stage } if backend_ranges::offers_ranges(&backend) => {
            answer(stages.finish(&stage).map(|()| Frame::Ok))
        }
        Frame::ListTolerant(path) => super::backend_ops::list_tolerant(sink, id, &backend, &path),
        Frame::WalkHashed2 {
            root,
            algorithm,
            min_bytes,
        } => super::backend_hash::walk_hashed2(
            sink, id, &backend, &root, algorithm, min_bytes, cancel,
        ),
        Frame::FindDuplicates { root, min_bytes } => {
            super::backend_ops::find_duplicates(sink, id, &backend, &root, min_bytes, cancel)
        }
        Frame::Query { kind, path } => answer(super::backend_ops::query(&backend, kind, &path)),
        Frame::RemoveTree(path) => {
            super::backend_remove_tree::remove_tree(sink, id, &backend, &path, cancel)
        }
        Frame::Recycle { path, size, sha256 } => {
            answer(super::backend_ops::recycle(&backend, &path, size, sha256))
        }
        Frame::FinishStage {
            stage,
            mtime_ms,
            mode,
            durability,
        } => answer(super::backend_ops::finish_stage(
            &backend, &stage, mtime_ms, mode, durability,
        )),
        request @ (Frame::RenameDirectoryNoReplace { .. }
        | Frame::CreateDirExclusive(_)
        | Frame::DirectoryIdentity(_)
        | Frame::FileIdentity(_)
        | Frame::DirectoryReservationSupported(_)) => emit(
            sink,
            id,
            &super::backend_ops::folder_operation(&backend, request, credit),
        ),
        Frame::Capacity(root) => answer(super::backend_ops::capacity(&backend, &root)),
        Frame::TargetLimits(root) => answer(Ok(super::backend_ops::target_limits(&backend, &root))),
        Frame::Watch { root, poll_ms } => {
            super::backend_ops::watch(sink, id, &backend, &root, poll_ms, cancel)
        }
        other => emit(
            sink,
            id,
            &Frame::Err(format!("unsupported request: {other:?}")),
        ),
    }
}

#[cfg(test)]
#[path = "backend_server_tests.rs"]
mod tests;

#[cfg(test)]
mod spawn_reply_tests {
    use super::*;

    #[test]
    fn speed_task_service_thread_refusal_replies_busy() {
        // `Builder::spawn` reports EAGAIN as WouldBlock
        let refused = io::Error::from_raw_os_error(libc_eagain());
        assert_eq!(refused.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(spawn_failure_reply(&refused), SpawnFailureReply::Busy);
        assert_eq!(
            spawn_failure_reply(&io::Error::from(io::ErrorKind::WouldBlock)),
            SpawnFailureReply::Busy
        );
        // the busy text is one that older clients take as overload
        assert!(THREAD_LIMIT_TEXT.contains("too many concurrent"));
        // every other reason stays an error of the request
        let other = io::Error::new(io::ErrorKind::OutOfMemory, "no stack");
        assert_eq!(
            spawn_failure_reply(&other),
            SpawnFailureReply::Failed("backend worker could not start: no stack".to_string())
        );
    }

    /// `EAGAIN` on Linux and Android, `WSAEWOULDBLOCK` on Windows.
    fn libc_eagain() -> i32 {
        if cfg!(windows) {
            10035
        } else {
            11
        }
    }
}
