use super::metadata::wire_to_vfs;
use super::mux::{make_out_channel, route_frame, Mux};
use crate::agent_proto::{self, Frame};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub struct AgentBackend {
    pub(super) inner: BackendHandle,
    pub(super) mux: Arc<Mux>,
    version: String,
}

impl AgentBackend {
    /// Hand-shake over an already-open framed stream pair.
    pub fn from_streams(
        r: Box<dyn Read + Send>,
        w: Box<dyn Write + Send>,
        inner: BackendHandle,
    ) -> io::Result<Self> {
        let (out_tx, out_rx) = make_out_channel();
        let pending = Arc::new(Mutex::new(HashMap::new()));

        // Writer thread: drain outgoing frames; closing the write half on exit
        // signals EOF to the agent.
        std::thread::Builder::new()
            .name("agent-writer".into())
            .spawn(move || {
                let mut w = w;
                while let Ok((id, frame)) = out_rx.recv() {
                    if agent_proto::write_frame(&mut w, id, &frame).is_err() {
                        break;
                    }
                }
            })
            .ok();

        // Reader thread: route inbound frames to the waiting op by req_id.
        let pending_r = pending.clone();
        std::thread::Builder::new()
            .name("agent-reader".into())
            .spawn(move || {
                let mut r = r;
                loop {
                    if !route_frame(&pending_r, agent_proto::read_frame(&mut r)) {
                        break;
                    }
                }
                // Drop all waiters so blocked recv() errors out and ops fall back.
                if let Ok(mut p) = pending_r.lock() {
                    p.clear();
                }
            })
            .ok();

        let mux = Arc::new(Mux::new(out_tx, pending));

        // Handshake before publishing the backend.
        let version = match mux.call(Frame::Hello {
            proto: agent_proto::PROTO_VERSION,
        })? {
            Frame::HelloOk { proto, version } if proto == agent_proto::PROTO_VERSION => version,
            Frame::HelloOk { proto, .. } => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("agent protocol {proto} != {}", agent_proto::PROTO_VERSION),
                ))
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected handshake reply: {other:?}"),
                ))
            }
        };
        Ok(AgentBackend {
            inner,
            mux,
            version,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

impl Backend for AgentBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match self.mux.call(Frame::ListDir(path.to_string())) {
            Ok(Frame::Dir(v)) => Ok(v.into_iter().map(wire_to_vfs).collect()),
            Ok(Frame::Err(e)) => Err(io::Error::other(e)),
            _ => self.inner.list_dir(path),
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match self.mux.call(Frame::Stat(path.to_string())) {
            Ok(Frame::Meta(m)) => Ok(wire_to_vfs(m)),
            Ok(Frame::Err(e)) => Err(io::Error::other(e)),
            _ => self.inner.stat(path),
        }
    }

    fn supports_walk_tree(&self) -> bool {
        true
    }

    fn walk_tree(
        &self,
        root: &str,
        on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
    ) -> Option<crate::agent_proto::WireNode> {
        let (id, rx) = self.mux.register();
        if self
            .mux
            .send(id, Frame::WalkTree(root.to_string()))
            .is_err()
        {
            self.mux.unregister(id);
            return None;
        }
        let mut last = (0u64, 0u64);
        let result = loop {
            match rx.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(Frame::Progress { done, total }) => {
                    last = (done, total);
                    if !on_progress(done, total) {
                        let _ = self.mux.send(id, Frame::Cancel);
                        break None;
                    }
                }
                Ok(Frame::Tree(n)) => break Some(n),
                Ok(Frame::Err(_)) => break None,
                Ok(_) => {}
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if !on_progress(last.0, last.1) {
                        let _ = self.mux.send(id, Frame::Cancel);
                        break None;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break None,
            }
        };
        self.mux.unregister(id);
        result
    }

    fn supports_bulk_tree(&self) -> bool {
        true
    }

    fn get_tree(&self, root: &str, dst: &Path) -> VfsResult<u64> {
        self.agent_get_tree(root, dst)
    }

    fn put_tree(&self, src: &Path, root: &str) -> VfsResult<u64> {
        self.agent_put_tree(src, root)
    }

    fn supports_search(&self) -> bool {
        true
    }

    fn search(
        &self,
        root: &str,
        spec: &crate::agent_proto::SearchSpec,
        tx: Sender<crate::vfs::SearchHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> bool {
        let (id, rx) = self.mux.register();
        if self
            .mux
            .send(
                id,
                Frame::Search {
                    root: root.to_string(),
                    spec: spec.clone(),
                },
            )
            .is_err()
        {
            self.mux.unregister(id);
            return false;
        }
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = self.mux.send(id, Frame::Cancel);
                break;
            }
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(Frame::Match {
                    rel,
                    is_dir,
                    size,
                    mtime_ms,
                }) => {
                    if tx
                        .send(crate::vfs::SearchHit {
                            rel,
                            is_dir,
                            size,
                            mtime_ms,
                        })
                        .is_err()
                    {
                        let _ = self.mux.send(id, Frame::Cancel);
                        break;
                    }
                }
                Ok(Frame::End) => break,
                Ok(Frame::Err(_)) => break,
                Ok(_) => {}
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
        self.mux.unregister(id);
        true
    }

    fn supports_walk_hashed(&self) -> bool {
        true
    }

    fn walk_hashed(
        &self,
        root: &str,
        want_hash: bool,
        tx: Sender<crate::vfs::HashHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> bool {
        let (id, rx) = self.mux.register();
        if self
            .mux
            .send(
                id,
                Frame::WalkHashed {
                    root: root.to_string(),
                    want_hash,
                },
            )
            .is_err()
        {
            self.mux.unregister(id);
            return false;
        }
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = self.mux.send(id, Frame::Cancel);
                break;
            }
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(Frame::HashEntry {
                    rel,
                    is_dir,
                    size,
                    mtime_ms,
                    md5,
                }) => {
                    if tx
                        .send(crate::vfs::HashHit {
                            rel,
                            is_dir,
                            size,
                            mtime_ms,
                            md5,
                        })
                        .is_err()
                    {
                        let _ = self.mux.send(id, Frame::Cancel);
                        break;
                    }
                }
                Ok(Frame::End) => break,
                Ok(Frame::Err(_)) => break,
                Ok(_) => {}
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
        self.mux.unregister(id);
        true
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        match self.agent_open_read(path) {
            Some(r) => Ok(r),
            None => self.inner.open_read(path),
        }
    }

    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        let _ = id;
        self.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        match self.agent_open_write(path) {
            Some(w) => Ok(w),
            None => self.inner.open_write(path),
        }
    }

    fn download_name(&self, path: &str, name: &str) -> String {
        self.inner.download_name(path, name)
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        match self.agent_unit_op(Frame::Copy {
            src: src.to_string(),
            dst: dst.to_string(),
        }) {
            Ok(true) => Ok(self.stat(dst).map(|m| m.size).unwrap_or(0)),
            Ok(false) => self.inner.copy_file(src, dst),
            Err(e) => Err(e),
        }
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        match self.agent_unit_op(Frame::Rename {
            src: src.to_string(),
            dst: dst.to_string(),
        }) {
            Ok(true) => Ok(()),
            Ok(false) => self.inner.rename(src, dst),
            Err(e) => Err(e),
        }
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        match self.agent_unit_op(Frame::Remove {
            path: path.to_string(),
            recursive: false,
        }) {
            Ok(true) => Ok(()),
            Ok(false) => self.inner.remove_file(path),
            Err(e) => Err(e),
        }
    }

    fn remove_file_id(&self, path: &str, _id: Option<&str>) -> VfsResult<()> {
        self.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        match self.agent_unit_op(Frame::Remove {
            path: path.to_string(),
            recursive: false,
        }) {
            Ok(true) => Ok(()),
            Ok(false) => self.inner.remove_dir(path),
            Err(e) => Err(e),
        }
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        match self.agent_unit_op(Frame::Mkdir(path.to_string())) {
            Ok(true) => Ok(()),
            Ok(false) => self.inner.mkdir_all(path),
            Err(e) => Err(e),
        }
    }

    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }

    fn rename_overwrites(&self) -> bool {
        true
    }

    fn is_local(&self) -> bool {
        self.inner.is_local()
    }

    fn provides_content_hash(&self) -> bool {
        self.inner.provides_content_hash()
    }
}
