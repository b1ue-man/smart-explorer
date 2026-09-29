use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::agent_proto::{
    self, busy_message, Frame, RequestContext, ServerSession, WireMeta, PROTO_VERSION,
};
use crate::vfs::{BackendHandle, VfsMeta};

use super::backend_batch::{handle_get_batch_backend, handle_put_batch_backend};
use super::backend_delete::remove_tree_backend;
use super::backend_stream::{handle_read_backend, handle_write_backend, WriteMode};
use super::backend_transfer::handle_put_tree_backend;
use super::backend_tree_send::handle_get_tree_backend;
use super::backend_walk::{
    handle_search_backend, handle_walk_hashed_backend, handle_walk_tree_backend, remove_one_backend,
};
use super::request_workers::{RequestWorkers, LEGACY_MAX_REQUEST_WORKERS};

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
        _ => error.to_string(),
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
/// transfer slots than the peer admits.
fn service_version(backend: &BackendHandle) -> String {
    let root = backend.root_display();
    let batch = backend.batch_limits(&root).is_some();
    let slots = agent_proto::service_slots(backend.transfer_ceiling(&root));
    format!("{} worker", agent_proto::server_version_with(batch, slots))
}

pub(crate) fn serve_backend(
    mut r: impl Read,
    w: impl Write + Send + 'static,
    backend: BackendHandle,
) -> io::Result<()> {
    let session = ServerSession::new(Arc::new(Mutex::new(Box::new(w))));
    let version = Arc::new(OnceLock::new());
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
        match std::thread::Builder::new()
            .name(format!("daemon-backend-request-{id}"))
            .spawn(move || {
                let result = dispatch_backend(&context, id, backend2, req, &version2, credit);
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
                emit(
                    session.sink(),
                    id,
                    &Frame::Err(format!("backend worker could not start: {error}")),
                )?;
            }
        }
    }
    session.abort_all();
    workers.shutdown()
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
    backend: BackendHandle,
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
            Some(rx) => {
                handle_write_backend(sink, id, &backend, &path, rx, cancel, WriteMode::Replace)
            }
            None => emit(sink, id, &Frame::Err("write: no inbound channel".into())),
        },
        Frame::WriteNew(path) => match inbound {
            Some(rx) => {
                handle_write_backend(sink, id, &backend, &path, rx, cancel, WriteMode::Create)
            }
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
                .server_copy_to_stage(&src, &stage, size)
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
            answer(backend.discard_copy_stage(&stage).map(|()| Frame::Ok))
        }
        other => emit(
            sink,
            id,
            &Frame::Err(format!("unsupported request: {other:?}")),
        ),
    }
}

fn vfs_to_wire(m: VfsMeta) -> WireMeta {
    WireMeta {
        name: m.name,
        is_dir: m.is_dir,
        is_symlink: m.is_symlink,
        size: m.size,
        mtime_ms: m.mtime_ms,
        content_md5: m.content_md5,
    }
}

#[cfg(test)]
#[path = "backend_server_tests.rs"]
mod tests;
