use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::agent_proto::{self, Frame, SearchSpec, WireMeta, WireNode, CHUNK, PROTO_VERSION};
use crate::vfs::{BackendHandle, VfsMeta};

use super::locks::lock_or_recover;

type Sink = Arc<Mutex<Box<dyn Write + Send>>>;

fn emit(sink: &Sink, id: u64, frame: &Frame) -> io::Result<()> {
    let mut w = sink
        .lock()
        .map_err(|_| io::Error::other("daemon backend writer locked"))?;
    agent_proto::write_frame(&mut *w, id, frame)
}

pub(crate) fn serve_backend(
    mut r: impl Read,
    w: impl Write + Send + 'static,
    backend: BackendHandle,
) -> io::Result<()> {
    let sink: Sink = Arc::new(Mutex::new(Box::new(w)));
    let inbound: Arc<Mutex<HashMap<u64, Sender<Frame>>>> = Arc::new(Mutex::new(HashMap::new()));
    let cancels: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>> = Arc::new(Mutex::new(HashMap::new()));

    while let Some((id, frame)) = agent_proto::read_frame(&mut r)? {
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
                if let Some(f) = lock_or_recover(&cancels).get(&id) {
                    f.store(true, Ordering::Relaxed);
                }
            }
            req => {
                let cancel = Arc::new(AtomicBool::new(false));
                lock_or_recover(&cancels).insert(id, cancel.clone());
                let rx = match &req {
                    Frame::Write(_) | Frame::PutTree(_) => {
                        let (tx, rx) = channel();
                        lock_or_recover(&inbound).insert(id, tx);
                        Some(rx)
                    }
                    _ => None,
                };
                let sink2 = sink.clone();
                let cancels2 = cancels.clone();
                let inbound2 = inbound.clone();
                let backend2 = backend.clone();
                std::thread::spawn(move || {
                    let res = dispatch_backend(&sink2, id, backend2, req, rx.as_ref(), &cancel);
                    if let Err(e) = res {
                        let _ = emit(&sink2, id, &Frame::Err(e.to_string()));
                    }
                    lock_or_recover(&cancels2).remove(&id);
                    lock_or_recover(&inbound2).remove(&id);
                });
            }
        }
    }
    Ok(())
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
        Frame::Remove { path, recursive } => {
            let res = if recursive {
                remove_tree_backend(&backend, &path)
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

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

fn rel_join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn node_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn handle_walk_tree_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let files = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let sink2 = sink.clone();
    let files2 = files.clone();
    let bytes2 = bytes.clone();
    let done2 = done.clone();
    let emitter = std::thread::spawn(move || {
        while !done2.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let _ = emit(
                &sink2,
                id,
                &Frame::Progress {
                    done: files2.load(Ordering::Relaxed),
                    total: bytes2.load(Ordering::Relaxed),
                },
            );
        }
    });
    let tree = walk_tree_node(backend, root, node_name(root), cancel, &files, &bytes)?;
    done.store(true, Ordering::Relaxed);
    let _ = emitter.join();
    emit(sink, id, &Frame::Tree(tree))
}

fn walk_tree_node(
    backend: &BackendHandle,
    path: &str,
    name: String,
    cancel: &AtomicBool,
    files: &AtomicU64,
    bytes: &AtomicU64,
) -> io::Result<WireNode> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(WireNode {
            name,
            size: 0,
            is_dir: true,
            children: Vec::new(),
        });
    }
    let meta = backend.stat(path)?;
    if !meta.is_dir {
        files.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(meta.size, Ordering::Relaxed);
        return Ok(WireNode {
            name,
            size: meta.size,
            is_dir: false,
            children: Vec::new(),
        });
    }
    let mut total = 0u64;
    let mut children = Vec::new();
    for child in backend.list_dir(path).unwrap_or_default() {
        if child.is_symlink {
            continue;
        }
        let child_path = join_path(path, &child.name);
        let node = walk_tree_node(
            backend,
            &child_path,
            child.name.clone(),
            cancel,
            files,
            bytes,
        )?;
        total = total.saturating_add(node.size);
        children.push(node);
    }
    Ok(WireNode {
        name,
        size: total,
        is_dir: true,
        children,
    })
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
    let mut w = backend.open_write(path)?;
    emit(sink, id, &Frame::Progress { done: 0, total: 0 })?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        match inbound.recv() {
            Ok(Frame::Data(d)) => w.write_all(&d)?,
            Ok(Frame::End) => break,
            Ok(_) => {}
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "daemon backend upload aborted",
                ))
            }
        }
    }
    w.flush()?;
    emit(sink, id, &Frame::Ok)
}

fn remove_one_backend(backend: &BackendHandle, path: &str) -> io::Result<()> {
    let meta = backend.stat(path)?;
    if meta.is_dir {
        backend.remove_dir(path)
    } else {
        backend.remove_file(path)
    }
}

fn remove_tree_backend(backend: &BackendHandle, path: &str) -> io::Result<()> {
    let meta = backend.stat(path)?;
    if !meta.is_dir {
        return backend.remove_file(path);
    }
    for child in backend.list_dir(path).unwrap_or_default() {
        let p = join_path(path, &child.name);
        if child.is_dir {
            remove_tree_backend(backend, &p)?;
        } else {
            backend.remove_file(&p)?;
        }
    }
    backend.remove_dir(path)
}

fn handle_get_tree_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    fn walk(
        sink: &Sink,
        id: u64,
        backend: &BackendHandle,
        path: &str,
        rel: &str,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        for child in backend.list_dir(path).unwrap_or_default() {
            if cancel.load(Ordering::Relaxed) || child.is_symlink {
                continue;
            }
            let child_path = join_path(path, &child.name);
            let child_rel = rel_join(rel, &child.name);
            emit(
                sink,
                id,
                &Frame::TreeEntry {
                    rel: child_rel.clone(),
                    is_dir: child.is_dir,
                    size: child.size,
                    mtime_ms: child.mtime_ms,
                },
            )?;
            if child.is_dir {
                walk(sink, id, backend, &child_path, &child_rel, cancel)?;
            } else {
                let mut r = backend.open_read(&child_path)?;
                let mut buf = vec![0u8; CHUNK];
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let n = r.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    emit(sink, id, &Frame::Data(buf[..n].to_vec()))?;
                }
            }
        }
        Ok(())
    }
    walk(sink, id, backend, root, "", cancel)?;
    emit(sink, id, &Frame::End)
}

fn handle_put_tree_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    backend.mkdir_all(root)?;
    let mut cur: Option<Box<dyn Write + Send>> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        match inbound.recv() {
            Ok(Frame::TreeEntry { rel, is_dir, .. }) => {
                if let Some(mut w) = cur.take() {
                    let _ = w.flush();
                }
                let dst = join_path(root, &rel);
                if is_dir {
                    backend.mkdir_all(&dst)?;
                } else {
                    if let Some(parent) = dst.rsplit_once('/').map(|(p, _)| p) {
                        if !parent.is_empty() {
                            backend.mkdir_all(parent)?;
                        }
                    }
                    cur = Some(backend.open_write(&dst)?);
                }
            }
            Ok(Frame::Data(d)) => {
                if let Some(w) = cur.as_mut() {
                    w.write_all(&d)?;
                }
            }
            Ok(Frame::End) => break,
            Ok(_) => {}
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "daemon backend put-tree aborted",
                ))
            }
        }
    }
    if let Some(mut w) = cur.take() {
        w.flush()?;
    }
    emit(sink, id, &Frame::Ok)
}

fn glob_match(pat: &str, s: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (
        pat.to_lowercase().chars().collect(),
        s.to_lowercase().chars().collect(),
    );
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn matches_spec(name: &str, is_dir: bool, size: u64, spec: &SearchSpec) -> bool {
    if is_dir && !spec.want_dirs {
        return false;
    }
    if !is_dir {
        if size < spec.min_size {
            return false;
        }
        if spec.max_size != 0 && size > spec.max_size {
            return false;
        }
    }
    if spec.query.is_empty() {
        return true;
    }
    if spec.glob {
        glob_match(&spec.query, name)
    } else {
        name.to_lowercase().contains(&spec.query.to_lowercase())
    }
}

fn handle_search_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    spec: &SearchSpec,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut count = 0u64;
    let mut stack = vec![(root.to_string(), String::new())];
    while let Some((dir, rel_dir)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        for child in backend.list_dir(&dir).unwrap_or_default() {
            if child.is_symlink {
                continue;
            }
            let rel = rel_join(&rel_dir, &child.name);
            if child.is_dir {
                stack.push((join_path(&dir, &child.name), rel.clone()));
            }
            if matches_spec(&child.name, child.is_dir, child.size, spec) {
                emit(
                    sink,
                    id,
                    &Frame::Match {
                        rel,
                        is_dir: child.is_dir,
                        size: child.size,
                        mtime_ms: child.mtime_ms,
                    },
                )?;
                count += 1;
                if spec.max_results != 0 && count >= spec.max_results {
                    return emit(sink, id, &Frame::End);
                }
            }
        }
    }
    emit(sink, id, &Frame::End)
}

fn handle_walk_hashed_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    want_hash: bool,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut stack = vec![(root.to_string(), String::new())];
    while let Some((dir, rel_dir)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        for child in backend.list_dir(&dir).unwrap_or_default() {
            if child.is_symlink {
                continue;
            }
            let path = join_path(&dir, &child.name);
            let rel = rel_join(&rel_dir, &child.name);
            if child.is_dir {
                emit(
                    sink,
                    id,
                    &Frame::HashEntry {
                        rel: rel.clone(),
                        is_dir: true,
                        size: 0,
                        mtime_ms: child.mtime_ms,
                        md5: None,
                    },
                )?;
                stack.push((path, rel));
            } else {
                let md5 = if want_hash {
                    child
                        .content_md5
                        .clone()
                        .or_else(|| md5_backend(backend, &path).ok())
                } else {
                    None
                };
                emit(
                    sink,
                    id,
                    &Frame::HashEntry {
                        rel,
                        is_dir: false,
                        size: child.size,
                        mtime_ms: child.mtime_ms,
                        md5,
                    },
                )?;
            }
        }
    }
    emit(sink, id, &Frame::End)
}

fn md5_backend(backend: &BackendHandle, path: &str) -> io::Result<String> {
    let mut r = backend.open_read(path)?;
    let mut ctx = md5::Context::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.consume(&buf[..n]);
    }
    Ok(format!("{:x}", ctx.compute()))
}

#[cfg(test)]
mod tests {
    use super::serve_backend;
    use crate::vfs::Backend;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

    #[test]
    fn backend_server_proxies_bulk_folder_transfer() {
        let base = std::env::temp_dir().join(format!(
            "se_daemon_backend_{}_{}",
            std::process::id(),
            crate::share::core_now_secs()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src/sub")).unwrap();
        std::fs::write(base.join("src/a.txt"), b"alpha").unwrap();
        std::fs::write(base.join("src/sub/b.txt"), b"bravo").unwrap();
        let root = base.to_string_lossy().replace('\\', "/");

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server_root = root.clone();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let read = sock.try_clone().unwrap();
            let backend: crate::vfs::BackendHandle =
                Arc::new(crate::vfs::LocalBackend::new(&server_root));
            serve_backend(read, sock, backend).unwrap();
        });

        let client = TcpStream::connect(addr).unwrap();
        let shut = client.try_clone().unwrap();
        let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
        let write: Box<dyn Write + Send> = Box::new(client);
        let inner: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new(&root));
        let be = crate::agent::AgentBackend::from_streams(read, write, inner).unwrap();

        assert!(be.supports_bulk_tree());
        let dst = format!("{root}/uploaded");
        assert_eq!(be.put_tree(&base.join("src"), &dst).unwrap(), 2);
        assert_eq!(
            std::fs::read(base.join("uploaded/sub/b.txt")).unwrap(),
            b"bravo"
        );
        let out = base.join("downloaded");
        assert_eq!(be.get_tree(&dst, &out).unwrap(), 2);
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"alpha");

        drop(be);
        let _ = shut.shutdown(std::net::Shutdown::Both);
        let _ = server.join();
        let _ = std::fs::remove_dir_all(base);
    }
}
