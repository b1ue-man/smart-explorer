use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::fs::{list_local, stat_local, try_exists_local, walk_dir_counted, WalkCounter};
use super::hash::handle_walk_hashed;
use super::put_tree::handle_put_tree;
use super::search::handle_search;
use super::session::{emit, Sink};
use super::transfer::{copy_file_safe, handle_get_tree, handle_read, handle_write, remove_path};
use super::{read_frame, Frame, PROTO_VERSION, TRANSFER_FRAME_BACKLOG};

type InboundSender = SyncSender<Frame>;
const MAX_ACTIVE_REQUESTS: usize = 8;
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
    let sink: Sink = Arc::new(Mutex::new(Box::new(w)));
    let inbound: Arc<Mutex<HashMap<u64, InboundSender>>> = Arc::new(Mutex::new(HashMap::new()));
    let cancels: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut workers = Vec::new();

    loop {
        if let Err(error) = reap_workers(&mut workers) {
            abort_requests(&inbound, &cancels);
            return match join_workers(&mut workers) {
                Ok(()) => Err(error),
                Err(shutdown) => Err(io::Error::other(format!(
                    "agent request worker failed ({error}); worker shutdown failed: {shutdown}"
                ))),
            };
        }
        let next = match read_frame(&mut r) {
            Ok(next) => next,
            Err(error) => {
                abort_requests(&inbound, &cancels);
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
                if workers.len() >= MAX_ACTIVE_REQUESTS {
                    emit(
                        &sink,
                        id,
                        &Frame::Err("too many concurrent agent requests".into()),
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
                match std::thread::Builder::new()
                    .name(format!("agent-request-{id}"))
                    .spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            dispatch(&sink2, id, req, rx.as_ref(), &cancel)
                        }));
                        match result {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                let _ = emit(&sink2, id, &Frame::Err(error.to_string()));
                            }
                            Err(_) => {
                                let _ = emit(
                                    &sink2,
                                    id,
                                    &Frame::Err("agent request worker panicked".to_string()),
                                );
                            }
                        }
                        lock_or_recover(&cancels2).remove(&id);
                        lock_or_recover(&inbound2).remove(&id);
                    }) {
                    Ok(worker) => workers.push(worker),
                    Err(error) => {
                        lock_or_recover(&cancels).remove(&id);
                        lock_or_recover(&inbound).remove(&id);
                        emit(
                            &sink,
                            id,
                            &Frame::Err(format!("request worker could not start: {error}")),
                        )?;
                    }
                }
            }
        }
    }
    abort_requests(&inbound, &cancels);
    join_workers(&mut workers)
}

fn dispatch(
    sink: &Sink,
    id: u64,
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
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        ),
        Frame::ListDir(p) => match list_local(&p) {
            Ok(v) => emit(sink, id, &Frame::Dir(v)),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Stat(p) => match stat_local(&p) {
            Ok(m) => emit(sink, id, &Frame::Meta(m)),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::TryExists(p) => match try_exists_local(&p) {
            Ok(exists) => emit(sink, id, &Frame::Exists(exists)),
            Err(error) => emit(sink, id, &Frame::Err(error.to_string())),
        },
        Frame::WalkTree(p) => handle_walk_tree(sink, id, &p, cancel),
        Frame::Read { path, offset, len } => handle_read(sink, id, &path, offset, len, cancel),
        Frame::Write(p) => match inbound {
            Some(rx) => handle_write(sink, id, &p, rx, cancel),
            None => emit(sink, id, &Frame::Err("write: no inbound channel".into())),
        },
        Frame::Copy { src, dst } => match copy_file_safe(&src, &dst, id) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Rename { src, dst } => match std::fs::rename(&src, &dst) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::RenameNoReplace { src, dst } => {
            match super::local_platform::rename_no_replace(Path::new(&src), Path::new(&dst)) {
                Ok(_) => emit(sink, id, &Frame::Ok),
                Err(error) => emit(sink, id, &Frame::Err(error.to_string())),
            }
        }
        Frame::Remove { path, recursive } => match remove_path(&path, recursive) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Mkdir(p) => match std::fs::create_dir_all(&p) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::GetTree(root) => handle_get_tree(sink, id, &root, cancel),
        Frame::PutTree(root) => match inbound {
            Some(rx) => handle_put_tree(sink, id, &root, rx, cancel),
            None => emit(sink, id, &Frame::Err("put-tree: no inbound channel".into())),
        },
        Frame::Search { root, spec } => handle_search(sink, id, &root, &spec, cancel),
        Frame::WalkHashed { root, want_hash } => {
            handle_walk_hashed(sink, id, &root, want_hash, cancel)
        }
        other => emit(
            sink,
            id,
            &Frame::Err(format!("unsupported request: {other:?}")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{cancel_request, join_workers, serve, transfer_channel};
    use crate::agent_proto::{write_frame, Frame, TRANSFER_FRAME_BACKLOG};
    use std::collections::HashMap;
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::TrySendError;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn inbound_transfer_channel_is_bounded_and_disconnects_both_ends() {
        let (sender, receiver) = transfer_channel();
        for _ in 0..TRANSFER_FRAME_BACKLOG {
            sender.try_send(Frame::Ok).unwrap();
        }
        assert!(matches!(
            sender.try_send(Frame::Ok),
            Err(TrySendError::Full(Frame::Ok))
        ));
        drop(receiver);
        assert!(matches!(
            sender.try_send(Frame::Ok),
            Err(TrySendError::Disconnected(Frame::Ok))
        ));

        let (sender, receiver) = transfer_channel();
        drop(sender);
        assert!(receiver.recv().is_err());
    }

    #[test]
    fn cancel_disconnects_a_blocked_transfer_receiver() {
        let (sender, receiver) = transfer_channel();
        let inbound = Mutex::new(HashMap::from([(9, sender)]));
        let cancel = Arc::new(AtomicBool::new(false));
        let cancels = Mutex::new(HashMap::from([(9, cancel.clone())]));

        cancel_request(9, &inbound, &cancels);

        assert!(cancel.load(Ordering::Relaxed));
        assert!(receiver.recv().is_err());
    }

    #[test]
    fn socket_disconnect_cancels_put_tree_and_preserves_destination() {
        let root = std::env::temp_dir().join(format!(
            "se_agent_server_disconnect_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let destination = root.join("file.txt");
        std::fs::write(&destination, b"old").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let read = socket.try_clone().unwrap();
            let _ = serve(read, socket);
        });
        let mut client = TcpStream::connect(address).unwrap();
        write_frame(
            &mut client,
            7,
            &Frame::PutTree(root.to_string_lossy().into_owned()),
        )
        .unwrap();
        write_frame(
            &mut client,
            7,
            &Frame::TreeEntry {
                rel: "file.txt".into(),
                is_dir: false,
                size: 3,
                mtime_ms: 0,
            },
        )
        .unwrap();
        write_frame(&mut client, 7, &Frame::Data(b"new".to_vec())).unwrap();
        client.shutdown(Shutdown::Both).unwrap();
        drop(client);
        server.join().unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
        assert!(!std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains(".se-agent-tree-")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn worker_shutdown_joins_cleanup_before_returning() {
        let cleaned = Arc::new(AtomicBool::new(false));
        let cleaned_worker = cleaned.clone();
        let mut workers = vec![std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            cleaned_worker.store(true, Ordering::Relaxed);
        })];
        join_workers(&mut workers).unwrap();
        assert!(workers.is_empty());
        assert!(cleaned.load(Ordering::Relaxed));
    }

    #[test]
    fn worker_shutdown_reports_panics() {
        let mut workers = vec![std::thread::spawn(|| panic!("request worker test panic"))];
        let error = join_workers(&mut workers).unwrap_err();
        assert!(error.to_string().contains("panicked"));
        assert!(workers.is_empty());
    }
}
