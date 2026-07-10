use super::metadata::wire_to_vfs;
use super::mux::{close_transport, make_out_channel, route_frame, Mux};
use crate::agent_proto::{self, Frame};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct AgentBackend {
    pub(super) inner: BackendHandle,
    pub(super) mux: Arc<Mux>,
    version: String,
}

fn operation_canceled(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, message)
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
        let closed = Arc::new(AtomicBool::new(false));
        let mux = Arc::new(Mux::new(out_tx, pending.clone(), closed.clone()));

        // Writer thread: drain outgoing frames; closing the write half on exit
        // signals EOF to the agent.
        let pending_w = pending.clone();
        let closed_w = closed.clone();
        std::thread::Builder::new()
            .name("agent-writer".into())
            .spawn(move || {
                let mut w = w;
                loop {
                    if closed_w.load(Ordering::Acquire) {
                        break;
                    }
                    match out_rx.recv_timeout(Duration::from_millis(100)) {
                        Ok((id, frame)) => {
                            if agent_proto::write_frame(&mut w, id, &frame).is_err() {
                                break;
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    }
                }
                close_transport(&closed_w, &pending_w);
            })?;

        // Reader thread: route inbound frames to the waiting op by req_id.
        let pending_r = pending.clone();
        let closed_r = closed.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("agent-reader".into())
            .spawn(move || {
                let mut r = r;
                loop {
                    if !route_frame(&pending_r, agent_proto::read_frame(&mut r)) {
                        break;
                    }
                }
                close_transport(&closed_r, &pending_r);
            })
        {
            close_transport(&closed, &pending);
            return Err(error);
        }

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

    fn state_identity(&self) -> String {
        self.inner.state_identity()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match self.mux.call(Frame::ListDir(path.to_string()))? {
            Frame::Dir(v) => Ok(v.into_iter().map(wire_to_vfs).collect()),
            Frame::Err(e) => Err(io::Error::other(e)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected agent directory reply: {other:?}"),
            )),
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match self.mux.call(Frame::Stat(path.to_string()))? {
            Frame::Meta(m) => Ok(wire_to_vfs(m)),
            Frame::Err(e) => Err(io::Error::other(e)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected agent metadata reply: {other:?}"),
            )),
        }
    }

    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        match self.mux.call(Frame::TryExists(path.to_string()))? {
            Frame::Exists(exists) => Ok(exists),
            Frame::Err(error) => Err(io::Error::other(error)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected agent existence reply: {other:?}"),
            )),
        }
    }

    fn supports_walk_tree(&self) -> bool {
        true
    }

    fn walk_tree(
        &self,
        root: &str,
        on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
    ) -> VfsResult<Option<crate::agent_proto::WireNode>> {
        let (id, rx) = self.mux.register();
        let result = (|| {
            self.mux.send(id, Frame::WalkTree(root.to_string()))?;
            let mut last = (0u64, 0u64);
            loop {
                match rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(Frame::Progress { done, total }) => {
                        last = (done, total);
                        if !on_progress(done, total) {
                            let _ = self.mux.send(id, Frame::Cancel);
                            return Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "agent tree walk canceled",
                            ));
                        }
                    }
                    Ok(Frame::Tree(node)) => return Ok(Some(node)),
                    Ok(Frame::Err(error)) => return Err(io::Error::other(error)),
                    Ok(other) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unexpected agent tree-walk reply: {other:?}"),
                        ));
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if !on_progress(last.0, last.1) {
                            let _ = self.mux.send(id, Frame::Cancel);
                            return Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "agent tree walk canceled",
                            ));
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent tree walk stream closed",
                        ));
                    }
                }
            }
        })();
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
    ) -> VfsResult<bool> {
        let (id, rx) = self.mux.register();
        let result = (|| {
            self.mux.send(
                id,
                Frame::Search {
                    root: root.to_string(),
                    spec: spec.clone(),
                },
            )?;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = self.mux.send(id, Frame::Cancel);
                    return Err(operation_canceled("agent search canceled"));
                }
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(Frame::Match {
                        rel,
                        is_dir,
                        size,
                        mtime_ms,
                    }) => tx
                        .send(crate::vfs::SearchHit {
                            rel,
                            is_dir,
                            size,
                            mtime_ms,
                        })
                        .map_err(|_| {
                            if cancel.load(Ordering::Relaxed) {
                                operation_canceled("agent search canceled")
                            } else {
                                io::Error::new(
                                    io::ErrorKind::BrokenPipe,
                                    "agent search result receiver closed",
                                )
                            }
                        })?,
                    Ok(Frame::End) => return Ok(true),
                    Ok(Frame::Err(error)) => return Err(io::Error::other(error)),
                    Ok(other) => {
                        let _ = self.mux.send(id, Frame::Cancel);
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unexpected agent search reply: {other:?}"),
                        ));
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent search stream closed",
                        ));
                    }
                }
            }
        })();
        if result.is_err() {
            let _ = self.mux.send(id, Frame::Cancel);
        }
        self.mux.unregister(id);
        result
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
    ) -> VfsResult<bool> {
        let (id, rx) = self.mux.register();
        let result = (|| {
            self.mux.send(
                id,
                Frame::WalkHashed {
                    root: root.to_string(),
                    want_hash,
                },
            )?;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = self.mux.send(id, Frame::Cancel);
                    return Err(operation_canceled("agent hash walk canceled"));
                }
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(Frame::HashEntry {
                        rel,
                        is_dir,
                        size,
                        mtime_ms,
                        md5,
                    }) => tx
                        .send(crate::vfs::HashHit {
                            rel,
                            is_dir,
                            size,
                            mtime_ms,
                            md5,
                        })
                        .map_err(|_| {
                            if cancel.load(Ordering::Relaxed) {
                                operation_canceled("agent hash walk canceled")
                            } else {
                                io::Error::new(
                                    io::ErrorKind::BrokenPipe,
                                    "agent hash-walk result receiver closed",
                                )
                            }
                        })?,
                    Ok(Frame::End) => return Ok(true),
                    Ok(Frame::Err(error)) => return Err(io::Error::other(error)),
                    Ok(other) => {
                        let _ = self.mux.send(id, Frame::Cancel);
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unexpected agent hash-walk reply: {other:?}"),
                        ));
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent hash-walk stream closed",
                        ));
                    }
                }
            }
        })();
        if result.is_err() {
            let _ = self.mux.send(id, Frame::Cancel);
        }
        self.mux.unregister(id);
        result
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.agent_open_read(path)
    }

    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        let _ = id;
        self.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.agent_open_write(path)
    }

    fn download_name(&self, path: &str, name: &str) -> String {
        self.inner.download_name(path, name)
    }

    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        self.agent_unit_op(Frame::Copy {
            src: src.to_string(),
            dst: dst.to_string(),
        })?;
        self.stat(dst).map(|meta| meta.size)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::Rename {
            src: src.to_string(),
            dst: dst.to_string(),
        })
    }

    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        match self.mux.call(Frame::RenameNoReplace {
            src: src.to_string(),
            dst: dst.to_string(),
        })? {
            Frame::Ok => Ok(()),
            Frame::Err(error) => Err(io::Error::other(error)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected agent no-replace reply: {other:?}"),
            )),
        }
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::Remove {
            path: path.to_string(),
            recursive: false,
        })
    }

    fn remove_file_id(&self, path: &str, _id: Option<&str>) -> VfsResult<()> {
        self.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::Remove {
            path: path.to_string(),
            recursive: false,
        })
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.agent_unit_op(Frame::Mkdir(path.to_string()))
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
