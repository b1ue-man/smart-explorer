use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use crate::agent_proto::{self, Frame, WireMeta, CHUNK, PROTO_VERSION, TRANSFER_FRAME_BACKLOG};
use crate::vfs::{BackendHandle, VfsMeta};

use super::backend_delete::remove_tree_backend;
use super::backend_transfer::handle_put_tree_backend;
use super::backend_tree_send::handle_get_tree_backend;
use super::backend_walk::{
    handle_search_backend, handle_walk_hashed_backend, handle_walk_tree_backend, remove_one_backend,
};
use super::locks::lock_or_recover;
use super::request_workers::RequestWorkers;

pub(super) type Sink = Arc<Mutex<Box<dyn Write + Send>>>;
type InboundSender = SyncSender<Frame>;

pub(super) fn emit(sink: &Sink, id: u64, frame: &Frame) -> io::Result<()> {
    let mut w = sink
        .lock()
        .map_err(|_| io::Error::other("daemon backend writer locked"))?;
    agent_proto::write_frame(&mut *w, id, frame)
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

fn abort_requests(
    inbound: &Mutex<HashMap<u64, InboundSender>>,
    cancels: &Mutex<HashMap<u64, Arc<AtomicBool>>>,
) {
    for cancel in lock_or_recover(cancels).values() {
        cancel.store(true, Ordering::Relaxed);
    }
    lock_or_recover(inbound).clear();
}

fn cancel_request(
    id: u64,
    inbound: &Mutex<HashMap<u64, InboundSender>>,
    cancels: &Mutex<HashMap<u64, Arc<AtomicBool>>>,
) {
    if let Some(cancel) = lock_or_recover(cancels).get(&id) {
        cancel.store(true, Ordering::Relaxed);
    }
    // Dropping the final sender wakes handlers currently blocked in recv.
    lock_or_recover(inbound).remove(&id);
}

fn transfer_channel() -> (InboundSender, Receiver<Frame>) {
    sync_channel(TRANSFER_FRAME_BACKLOG)
}

pub(crate) fn serve_backend(
    mut r: impl Read,
    w: impl Write + Send + 'static,
    backend: BackendHandle,
) -> io::Result<()> {
    let sink: Sink = Arc::new(Mutex::new(Box::new(w)));
    let inbound: Arc<Mutex<HashMap<u64, InboundSender>>> = Arc::new(Mutex::new(HashMap::new()));
    let cancels: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut workers = RequestWorkers::default();

    loop {
        let next = match agent_proto::read_frame(&mut r) {
            Ok(next) => next,
            Err(error) => {
                abort_requests(&inbound, &cancels);
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
        match frame {
            Frame::Data(_) | Frame::TreeEntry { .. } | Frame::End => {
                let tx = lock_or_recover(&inbound).get(&id).cloned();
                if let Some(tx) = tx {
                    let is_end = matches!(frame, Frame::End);
                    let _ = tx.send(frame);
                    if is_end {
                        lock_or_recover(&inbound).remove(&id);
                    }
                }
            }
            Frame::Cancel => {
                cancel_request(id, &inbound, &cancels);
            }
            req => {
                let has_capacity = match workers.has_capacity() {
                    Ok(has_capacity) => has_capacity,
                    Err(worker_error) => {
                        abort_requests(&inbound, &cancels);
                        return match workers.shutdown() {
                            Ok(()) => Err(worker_error),
                            Err(shutdown_error) => Err(io::Error::other(format!(
                                "backend request worker failed ({worker_error}); worker shutdown failed: {shutdown_error}"
                            ))),
                        };
                    }
                };
                if !has_capacity {
                    emit(
                        &sink,
                        id,
                        &Frame::Err("too many concurrent backend requests".into()),
                    )?;
                    continue;
                }
                if lock_or_recover(&cancels).contains_key(&id) {
                    emit(&sink, id, &Frame::Err("duplicate active request id".into()))?;
                    continue;
                }
                let cancel = Arc::new(AtomicBool::new(false));
                lock_or_recover(&cancels).insert(id, cancel.clone());
                let rx = match &req {
                    Frame::Write(_) | Frame::PutTree(_) => {
                        let (tx, rx) = transfer_channel();
                        lock_or_recover(&inbound).insert(id, tx);
                        Some(rx)
                    }
                    _ => None,
                };
                let sink2 = sink.clone();
                let cancels2 = cancels.clone();
                let inbound2 = inbound.clone();
                let backend2 = backend.clone();
                match std::thread::Builder::new()
                    .name(format!("daemon-backend-request-{id}"))
                    .spawn(move || {
                        let result = dispatch_backend(&sink2, id, backend2, req, rx.as_ref(), &cancel);
                        let result = match result {
                            Ok(()) => Ok(()),
                            Err(error) => match emit(&sink2, id, &Frame::Err(error.to_string())) {
                                // The request failure has been surfaced to the
                                // client; only a reporting failure remains a
                                // worker-level error that tears down the link.
                                Ok(()) => Ok(()),
                                Err(report_error)
                                    if canceled_request_lost_client(
                                        &error,
                                        &report_error,
                                        cancel.load(Ordering::Relaxed),
                                    ) =>
                                {
                                    Ok(())
                                }
                                Err(report_error) => Err(io::Error::new(
                                    error.kind(),
                                    format!("request failed ({error}); reporting it failed: {report_error}"),
                                )),
                            },
                        };
                        lock_or_recover(&cancels2).remove(&id);
                        lock_or_recover(&inbound2).remove(&id);
                        result
                    }) {
                    Ok(worker) => workers.push(worker),
                    Err(error) => {
                        lock_or_recover(&cancels).remove(&id);
                        lock_or_recover(&inbound).remove(&id);
                        emit(
                            &sink,
                            id,
                            &Frame::Err(format!("backend worker could not start: {error}")),
                        )?;
                    }
                }
            }
        }
    }
    abort_requests(&inbound, &cancels);
    workers.shutdown()
}

fn dispatch_backend(
    sink: &Sink,
    id: u64,
    backend: BackendHandle,
    req: Frame,
    inbound: Option<&Receiver<Frame>>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    match req {
        Frame::Hello { .. } => emit(
            sink,
            id,
            &Frame::HelloOk {
                proto: PROTO_VERSION,
                version: format!("{} worker", env!("CARGO_PKG_VERSION")),
            },
        ),
        Frame::ListDir(p) => match backend.list_dir(&p) {
            Ok(v) => emit(
                sink,
                id,
                &Frame::Dir(v.into_iter().map(vfs_to_wire).collect()),
            ),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Stat(p) => match backend.stat(&p) {
            Ok(m) => emit(sink, id, &Frame::Meta(vfs_to_wire(m))),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::TryExists(p) => match backend.try_exists(&p) {
            Ok(exists) => emit(sink, id, &Frame::Exists(exists)),
            Err(error) => emit(sink, id, &Frame::Err(error.to_string())),
        },
        Frame::WalkTree(root) => handle_walk_tree_backend(sink, id, &backend, &root, cancel),
        Frame::Read { path, offset, len } => {
            handle_read_backend(sink, id, &backend, &path, offset, len, cancel)
        }
        Frame::Write(path) => match inbound {
            Some(rx) => handle_write_backend(sink, id, &backend, &path, rx, cancel),
            None => emit(sink, id, &Frame::Err("write: no inbound channel".into())),
        },
        Frame::Copy { src, dst } => match backend.copy_file(&src, &dst) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Rename { src, dst } => match backend.rename(&src, &dst) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::RenameNoReplace { src, dst } => match backend.rename_no_replace(&src, &dst) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(error) => emit(sink, id, &Frame::Err(error.to_string())),
        },
        Frame::Remove { path, recursive } => {
            let res = if recursive {
                remove_tree_backend(&backend, &path, cancel)
            } else {
                remove_one_backend(&backend, &path)
            };
            match res {
                Ok(_) => emit(sink, id, &Frame::Ok),
                Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
            }
        }
        Frame::Mkdir(p) => match backend.mkdir_all(&p) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
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
    }
}

fn handle_read_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    path: &str,
    offset: u64,
    len: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut r = backend.open_read(path)?;
    if offset > 0 {
        let mut skip = (&mut r).take(offset);
        io::copy(&mut skip, &mut io::sink())?;
    }
    let mut remaining = if len == 0 { u64::MAX } else { len };
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let want = remaining.min(buf.len() as u64) as usize;
        let n = r.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        emit(sink, id, &Frame::Data(buf[..n].to_vec()))?;
        remaining -= n as u64;
    }
    emit(sink, id, &Frame::End)
}

fn handle_write_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    path: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let staged = crate::vfs::unique_staging_path(&**backend, path, "daemon")?;
    let mut writer = backend.open_write(&staged)?;
    emit(sink, id, &Frame::Progress { done: 0, total: 0 })?;
    let transfer = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "upload canceled",
            ));
        }
        match inbound.recv() {
            Ok(Frame::Data(data)) => {
                if let Err(error) = writer.write_all(&data) {
                    break Err(error);
                }
            }
            Ok(Frame::End) => break writer.flush(),
            Ok(_) => {}
            Err(_) => {
                break Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "daemon backend upload aborted",
                ));
            }
        }
    };
    drop(writer);
    if let Err(error) = transfer {
        let _ = backend.remove_file(&staged);
        return Err(error);
    }
    if let Err(error) = crate::vfs::promote_staged_replace(&**backend, &staged, path) {
        let _ = backend.remove_file(&staged);
        return Err(error);
    }
    emit(sink, id, &Frame::Ok)
}

#[cfg(test)]
#[path = "backend_server_tests.rs"]
mod tests;
