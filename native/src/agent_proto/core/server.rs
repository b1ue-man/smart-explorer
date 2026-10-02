use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::batch_get::handle_get_batch;
use super::batch_put::handle_put_batch;
use super::fs::{list_local, stat_local, try_exists_local, walk_dir_counted, WalkCounter};
use super::hash::handle_walk_hashed;
use super::put_tree::handle_put_tree;
use super::search::handle_search;
use super::server_session::{RequestContext, ServerSession};
use super::session::{emit, Sink};
use super::stage_ops::{copy_to_stage, create_dir_one, discard_stage};
use super::transfer::{copy_file_safe, handle_get_tree, handle_read, handle_write, remove_path};
use super::write_new::handle_write_new;
use super::{read_frame, Frame, PROTO_VERSION};

/// Requests a connection without credit admits: the former limit, since
/// each such request may buffer a full 32-frame upload queue.
const LEGACY_MAX_ACTIVE_REQUESTS: usize = 8;
/// Requests a credit connection admits (connection budget / initial credit).
const MAX_ACTIVE_REQUESTS: usize = super::CREDIT_REQUEST_LIMIT;
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

fn reap_workers(workers: &mut Vec<std::thread::JoinHandle<()>>) -> io::Result<()> {
    let mut index = 0;
    let mut panicked = 0usize;
    while index < workers.len() {
        if workers[index].is_finished() {
            let worker = workers.swap_remove(index);
            if worker.join().is_err() {
                panicked = panicked.saturating_add(1);
            }
        } else {
            index += 1;
        }
    }
    if panicked == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{panicked} agent request worker(s) panicked"
        )))
    }
}

fn join_workers(workers: &mut Vec<std::thread::JoinHandle<()>>) -> io::Result<()> {
    let deadline = Instant::now() + WORKER_SHUTDOWN_TIMEOUT;
    let mut worker_failure: Option<io::Error> = None;
    while !workers.is_empty() {
        if let Err(error) = reap_workers(workers) {
            worker_failure.get_or_insert(error);
        }
        if workers.is_empty() {
            return worker_failure.map_or(Ok(()), Err);
        }
        if Instant::now() >= deadline {
            let timeout = io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{} agent request worker(s) did not stop after cancellation",
                    workers.len()
                ),
            );
            return match worker_failure {
                Some(failure) => Err(io::Error::new(
                    timeout.kind(),
                    format!("{failure}; {timeout}"),
                )),
                None => Err(timeout),
            };
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn handle_walk_tree(sink: &Sink, id: u64, root: &str, cancel: &AtomicBool) -> io::Result<()> {
    let p = Path::new(root);
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string());
    let cnt = Arc::new(WalkCounter::new());
    let done = Arc::new(AtomicBool::new(false));
    let sink2 = sink.clone();
    let cnt2 = cnt.clone();
    let done2 = done.clone();
    let emitter = std::thread::Builder::new()
        .name("agent-walk-progress".into())
        .spawn(move || {
            while !done2.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(200));
                let f = cnt2.files.load(Ordering::Relaxed);
                let b = cnt2.bytes.load(Ordering::Relaxed);
                if emit(&sink2, id, &Frame::Progress { done: f, total: b }).is_err() {
                    break;
                }
            }
        })?;
    let tree = walk_dir_counted(p, name, &cnt, cancel);
    done.store(true, Ordering::Relaxed);
    if emitter.join().is_err() {
        return match tree {
            Ok(_) => Err(io::Error::other("agent walk progress worker panicked")),
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!("{error}; agent walk progress worker panicked"),
            )),
        };
    }
    emit(sink, id, &Frame::Tree(tree?))
}

/// Drive the agent request loop.
pub fn serve(mut r: impl Read, w: impl Write + Send + 'static) -> io::Result<()> {
    let session = ServerSession::new(Arc::new(Mutex::new(Box::new(w))));
    let mut workers = Vec::new();

    loop {
        let next = match read_frame(&mut r) {
            Ok(next) => next,
            Err(error) => {
                session.abort_all();
                return match join_workers(&mut workers) {
                    Ok(()) => Err(error),
                    Err(shutdown) => Err(io::Error::new(
                        error.kind(),
                        format!("agent input failed ({error}); worker shutdown failed: {shutdown}"),
                    )),
                };
            }
        };
        let Some((id, frame)) = next else {
            break;
        };
        // Input can block while every previous request finishes. Capacity
        // must reflect completion after that wait, not the preceding frame.
        if let Err(error) = reap_workers(&mut workers) {
            session.abort_all();
            return match join_workers(&mut workers) {
                Ok(()) => Err(error),
                Err(shutdown) => Err(io::Error::other(format!(
                    "agent request worker failed ({error}); worker shutdown failed: {shutdown}"
                ))),
            };
        }
        let Some(request) = session.route(id, frame) else {
            continue;
        };
        if workers.len() >= session.request_limit(LEGACY_MAX_ACTIVE_REQUESTS) {
            session.reject_busy(id, "too many concurrent agent requests")?;
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
        let context = session.open(id, &request);
        let worker_session = session.clone();
        match std::thread::Builder::new()
            .name(format!("agent-request-{id}"))
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    dispatch(&context, id, request)
                }));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        let _ = emit(&context.sink, id, &Frame::Err(error.to_string()));
                    }
                    Err(_) => {
                        let _ = emit(
                            &context.sink,
                            id,
                            &Frame::Err("agent request worker panicked".to_string()),
                        );
                    }
                }
                worker_session.close(id);
            }) {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                session.discard(id);
                emit(
                    session.sink(),
                    id,
                    &Frame::Err(format!("request worker could not start: {error}")),
                )?;
            }
        }
    }
    session.abort_all();
    join_workers(&mut workers)
}

fn reply(sink: &Sink, id: u64, result: io::Result<Frame>) -> io::Result<()> {
    match result {
        Ok(frame) => emit(sink, id, &frame),
        Err(error) => emit(sink, id, &Frame::Err(error.to_string())),
    }
}

fn dispatch(context: &RequestContext, id: u64, req: Frame) -> io::Result<()> {
    let sink = &context.sink;
    let cancel: &AtomicBool = &context.cancel;
    let inbound = context.inbound.as_deref();
    match req {
        Frame::Hello { .. } => emit(
            sink,
            id,
            &Frame::HelloOk {
                proto: PROTO_VERSION,
                version: super::server_version(true),
            },
        ),
        Frame::ListDir(p) => reply(sink, id, list_local(&p).map(Frame::Dir)),
        Frame::Stat(p) => reply(sink, id, stat_local(&p).map(Frame::Meta)),
        Frame::TryExists(p) => reply(sink, id, try_exists_local(&p).map(Frame::Exists)),
        Frame::WalkTree(p) => handle_walk_tree(sink, id, &p, cancel),
        Frame::Read { path, offset, len } => handle_read(sink, id, &path, offset, len, cancel),
        Frame::Write(p) => match inbound {
            Some(rx) => handle_write(
                sink,
                id,
                &p,
                rx,
                cancel,
                super::promotion::promote_staged_replace,
            ),
            None => emit(sink, id, &Frame::Err("write: no inbound channel".into())),
        },
        Frame::WriteNew(p) => match inbound {
            Some(rx) => handle_write_new(sink, id, &p, rx, cancel),
            None => emit(
                sink,
                id,
                &Frame::Err("write-new: no inbound channel".into()),
            ),
        },
        Frame::Copy { src, dst } => {
            reply(sink, id, copy_file_safe(&src, &dst, id).map(|_| Frame::Ok))
        }
        Frame::Rename { src, dst } => {
            reply(sink, id, std::fs::rename(&src, &dst).map(|()| Frame::Ok))
        }
        Frame::RenameNoReplace { src, dst } => reply(
            sink,
            id,
            super::local_platform::rename_no_replace(Path::new(&src), Path::new(&dst))
                .map(|()| Frame::Ok),
        ),
        Frame::Promote {
            staged,
            destination,
        } => reply(
            sink,
            id,
            super::promotion::promote_staged_replace(Path::new(&staged), Path::new(&destination))
                .map(|()| Frame::Ok),
        ),
        Frame::PromoteNoReplace {
            staged,
            destination,
        } => reply(
            sink,
            id,
            super::promotion::promote_staged_no_replace(
                Path::new(&staged),
                Path::new(&destination),
            )
            .map(|()| Frame::Ok),
        ),
        Frame::Remove { path, recursive } => {
            reply(sink, id, remove_path(&path, recursive).map(|()| Frame::Ok))
        }
        Frame::Mkdir(p) => reply(sink, id, std::fs::create_dir_all(&p).map(|()| Frame::Ok)),
        Frame::GetTree(root) => handle_get_tree(sink, id, &root, cancel),
        Frame::PutTree(root) => match inbound {
            Some(rx) => handle_put_tree(sink, id, &root, rx, cancel),
            None => emit(sink, id, &Frame::Err("put-tree: no inbound channel".into())),
        },
        Frame::Search { root, spec } => handle_search(sink, id, &root, &spec, cancel),
        Frame::WalkHashed { root, want_hash } => {
            handle_walk_hashed(sink, id, &root, want_hash, cancel)
        }
        Frame::BatchPut { entries } => match inbound {
            Some(rx) => handle_put_batch(sink, id, &entries, rx, cancel),
            None => emit(sink, id, &Frame::Err("batch: no inbound channel".into())),
        },
        Frame::BatchGet { items } => handle_get_batch(sink, id, &items, cancel),
        Frame::CopyToStage { src, stage, size } => reply(
            sink,
            id,
            copy_to_stage(&src, &stage, size, cancel).map(|copied| Frame::Copied(Some(copied))),
        ),
        Frame::CreateDir { path, exclusive } => reply(
            sink,
            id,
            create_dir_one(&path, exclusive).map(|()| Frame::Ok),
        ),
        Frame::DiscardStage(path) => reply(sink, id, discard_stage(&path).map(|()| Frame::Ok)),
        Frame::ListTolerant(path) => super::ext_ops::handle_list_tolerant(sink, id, &path, cancel),
        Frame::WalkHashed2 {
            root,
            algorithm,
            min_bytes,
        } => super::ext_ops::handle_walk_hashed2(sink, id, &root, algorithm, min_bytes, cancel),
        Frame::FinishStage {
            stage,
            mtime_ms,
            mode,
            durability,
        } => reply(
            sink,
            id,
            super::ext_ops::finish_stage(&stage, mtime_ms, mode, durability),
        ),
        Frame::Query { kind, path } => reply(sink, id, super::ext_ops::answer(kind, &path)),
        Frame::FindDuplicates { .. }
        | Frame::Recycle { .. }
        | Frame::TargetLimits(_)
        | Frame::Watch { .. } => emit(
            sink,
            id,
            &Frame::Err(super::UNSUPPORTED_EXTENSION.to_string()),
        ),
        other => emit(
            sink,
            id,
            &Frame::Err(format!("unsupported request: {other:?}")),
        ),
    }
}

#[cfg(test)]
#[path = "server_bulk_task_tests.rs"]
mod bulk_task_tests;

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
