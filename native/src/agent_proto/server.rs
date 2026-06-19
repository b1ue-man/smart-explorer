use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use super::core_oslocked::{list_local, stat_local, walk_dir_counted, WalkCounter};
use super::hash::handle_walk_hashed;
use super::search::handle_search;
use super::session::{emit, Sink};
use super::transfer::{handle_get_tree, handle_put_tree, handle_read, handle_write, remove_path};
use super::{read_frame, Frame, PROTO_VERSION};

fn handle_walk_tree(sink: &Sink, id: u64, root: &str, cancel: &AtomicBool) -> io::Result<()> {
    let p = Path::new(root);
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string());
    let cnt = Arc::new(WalkCounter {
        files: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
    });
    let done = Arc::new(AtomicBool::new(false));
    let sink2 = sink.clone();
    let cnt2 = cnt.clone();
    let done2 = done.clone();
    let emitter = std::thread::spawn(move || {
        while !done2.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let f = cnt2.files.load(Ordering::Relaxed);
            let b = cnt2.bytes.load(Ordering::Relaxed);
            if emit(&sink2, id, &Frame::Progress { done: f, total: b }).is_err() {
                break;
            }
        }
    });
    let tree = walk_dir_counted(p, name, &cnt, cancel);
    done.store(true, Ordering::Relaxed);
    let _ = emitter.join();
    emit(sink, id, &Frame::Tree(tree))
}

/// Drive the agent request loop.
pub fn serve(mut r: impl Read, w: impl Write + Send + 'static) -> io::Result<()> {
    let sink: Sink = Arc::new(Mutex::new(Box::new(w)));
    let inbound: Arc<Mutex<HashMap<u64, Sender<Frame>>>> = Arc::new(Mutex::new(HashMap::new()));
    let cancels: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>> = Arc::new(Mutex::new(HashMap::new()));

    while let Some((id, frame)) = read_frame(&mut r)? {
        match frame {
            Frame::Data(_) | Frame::TreeEntry { .. } | Frame::End => {
                let tx = inbound.lock().unwrap().get(&id).cloned();
                if let Some(tx) = tx {
                    let is_end = matches!(frame, Frame::End);
                    let _ = tx.send(frame);
                    if is_end {
                        inbound.lock().unwrap().remove(&id);
                    }
                }
            }
            Frame::Cancel => {
                if let Some(f) = cancels.lock().unwrap().get(&id) {
                    f.store(true, Ordering::Relaxed);
                }
            }
            req => {
                let cancel = Arc::new(AtomicBool::new(false));
                cancels.lock().unwrap().insert(id, cancel.clone());
                let rx = match &req {
                    Frame::Write(_) | Frame::PutTree(_) => {
                        let (tx, rx) = channel();
                        inbound.lock().unwrap().insert(id, tx);
                        Some(rx)
                    }
                    _ => None,
                };
                let sink2 = sink.clone();
                let cancels2 = cancels.clone();
                let inbound2 = inbound.clone();
                std::thread::spawn(move || {
                    let res = dispatch(&sink2, id, req, rx.as_ref(), &cancel);
                    if let Err(e) = res {
                        let _ = emit(&sink2, id, &Frame::Err(e.to_string()));
                    }
                    cancels2.lock().unwrap().remove(&id);
                    inbound2.lock().unwrap().remove(&id);
                });
            }
        }
    }
    Ok(())
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
        Frame::WalkTree(p) => handle_walk_tree(sink, id, &p, cancel),
        Frame::Read { path, offset, len } => handle_read(sink, id, &path, offset, len, cancel),
        Frame::Write(p) => match inbound {
            Some(rx) => handle_write(sink, id, &p, rx, cancel),
            None => emit(sink, id, &Frame::Err("write: no inbound channel".into())),
        },
        Frame::Copy { src, dst } => match std::fs::copy(&src, &dst) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
        Frame::Rename { src, dst } => match std::fs::rename(&src, &dst) {
            Ok(_) => emit(sink, id, &Frame::Ok),
            Err(e) => emit(sink, id, &Frame::Err(e.to_string())),
        },
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
