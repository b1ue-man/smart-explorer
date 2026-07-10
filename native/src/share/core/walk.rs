use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::{RecvStream, SendStream};

use super::backend::{recv_resp, reply, send_ctrl, PeerBackend};
use super::core::eio;
use super::fs::{self, ShareExportConfig};
use super::io_deadline;
use super::walk_assembly::{
    invalid, validate_name, TreeAssembler, WalkTotals, MAX_WALK_DEPTH, MAX_WALK_NAME_BYTES,
    MAX_WALK_NODES, WALK_BATCH_NODES,
};
use super::wire::{Ctrl, FsRequest, FsResponse, FsWalkNode};

const CANCEL_POLL: Duration = Duration::from_millis(250);
const WALK_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Run the PeerBackend's one-stream analytics walk. Every protocol, transport,
/// cancellation, and validation failure remains an error; `Ok(None)` is
/// reserved by the backend contract for implementations that are unsupported.
pub(super) fn walk_peer(
    backend: &PeerBackend,
    root: &str,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> io::Result<Option<crate::agent_proto::WireNode>> {
    if !on_progress(0, 0) {
        return Err(interrupted());
    }
    let (send, recv) = backend
        .node
        .open_stream(&backend.endpoint, &backend.identity)?;
    backend
        .node
        .block_on(receive_walk(send, recv, root, on_progress))
        .map(Some)
}

async fn receive_walk(
    mut send: SendStream,
    mut recv: RecvStream,
    root: &str,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> io::Result<crate::agent_proto::WireNode> {
    io_deadline::run(
        "peer tree request",
        send_ctrl(
            &mut send,
            &Ctrl::Fs {
                req: FsRequest::WalkTree {
                    path: root.to_string(),
                },
            },
        ),
    )
    .await?;

    let result = receive_walk_responses(&mut recv, on_progress).await;
    if result.is_err() {
        // STOP_SENDING immediately releases the server's flow-controlled walk;
        // reset the unused request half as well so cancellation is unambiguous.
        io_deadline::abort(&mut send, &mut recv);
    }
    result
}

async fn receive_walk_responses(
    recv: &mut RecvStream,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> io::Result<crate::agent_proto::WireNode> {
    let mut tree = TreeAssembler::default();
    let mut progress = (0u64, 0u64);
    loop {
        let resp = recv_response_checked(recv, progress, on_progress).await?;
        match resp {
            FsResponse::WalkBatch {
                nodes,
                files,
                dirs,
                bytes,
            } => {
                tree.push_batch(nodes, WalkTotals { files, dirs, bytes })?;
                progress = (files, bytes);
                if !on_progress(files, bytes) {
                    return Err(interrupted());
                }
            }
            FsResponse::WalkDone {
                files,
                dirs,
                bytes,
                nodes,
            } => {
                if !on_progress(files, bytes) {
                    return Err(interrupted());
                }
                return tree.finish(WalkTotals { files, dirs, bytes }, nodes);
            }
            _ => return Err(invalid("unexpected response in peer tree walk")),
        }
    }
}

async fn recv_response_checked(
    recv: &mut RecvStream,
    progress: (u64, u64),
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> io::Result<FsResponse> {
    // Keep the same read future pinned across timer ticks: RecvStream's
    // read_exact is not cancellation-safe once it has consumed a frame prefix.
    let response = recv_resp(recv);
    tokio::pin!(response);
    let idle = tokio::time::sleep(WALK_IDLE_TIMEOUT);
    tokio::pin!(idle);
    let mut poll = tokio::time::interval(CANCEL_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    poll.tick().await;
    loop {
        tokio::select! {
            biased;
            response = &mut response => return response,
            _ = poll.tick() => {
                if !on_progress(progress.0, progress.1) {
                    return Err(interrupted());
                }
            }
            _ = &mut idle => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "peer tree walk made no progress for 60 seconds",
                ));
            }
        }
    }
}

/// Serve a walk only through the virtual share filesystem. `fs::stat` and
/// `fs::list_dir` resolve every directory against the authenticated export
/// roots, so no client-provided native path reaches a backend directly.
pub(super) async fn serve_walk(
    send: &mut SendStream,
    root: String,
    exports: Arc<Mutex<ShareExportConfig>>,
) -> io::Result<()> {
    let mut walker = match ServerWalker::new(root, exports) {
        Ok(walker) => walker,
        Err(error) => return reply_walk(send, super::fs_error::response(&error)).await,
    };
    let mut batch = NodeBatch::default();
    let mut totals = WalkTotals::default();
    let mut last_send = Instant::now();

    loop {
        let event = match walker.next_event() {
            Ok(event) => event,
            Err(error) => return reply_walk(send, super::fs_error::response(&error)).await,
        };
        match event {
            Some(WalkEvent::Node(node)) => {
                if let Err(error) = totals.observe(&node) {
                    return reply_walk(send, super::fs_error::response(&error)).await;
                }
                if let Some(nodes) = batch.push(node) {
                    send_batch(send, nodes, totals).await?;
                    last_send = Instant::now();
                }
            }
            Some(WalkEvent::Checkpoint) if last_send.elapsed() >= CANCEL_POLL => {
                send_batch(send, batch.take(), totals).await?;
                last_send = Instant::now();
            }
            Some(WalkEvent::Checkpoint) => {}
            None => {
                if !batch.is_empty() {
                    send_batch(send, batch.take(), totals).await?;
                }
                return reply_walk(
                    send,
                    FsResponse::WalkDone {
                        files: totals.files,
                        dirs: totals.dirs,
                        bytes: totals.bytes,
                        nodes: totals.nodes(),
                    },
                )
                .await;
            }
        }
    }
}

async fn send_batch(
    send: &mut SendStream,
    nodes: Vec<FsWalkNode>,
    totals: WalkTotals,
) -> io::Result<()> {
    reply_walk(
        send,
        FsResponse::WalkBatch {
            nodes,
            files: totals.files,
            dirs: totals.dirs,
            bytes: totals.bytes,
        },
    )
    .await
}

async fn reply_walk(send: &mut SendStream, response: FsResponse) -> io::Result<()> {
    io_deadline::run("peer tree response", reply(send, response)).await
}

#[derive(Clone)]
struct DirSeed {
    id: u64,
    parent: Option<u64>,
    depth: usize,
    name: String,
    parent_path: Arc<str>,
}

impl DirSeed {
    fn path(&self) -> String {
        if self.name == "/" {
            "/".into()
        } else {
            child_path(&self.parent_path, &self.name)
        }
    }
}

enum WalkWork {
    Enter(DirSeed),
    Entries {
        dir: DirSeed,
        path: Arc<str>,
        entries: std::vec::IntoIter<super::wire::FsMeta>,
        subdirs: Vec<DirSeed>,
    },
    Descend {
        dir: DirSeed,
        subdirs: std::vec::IntoIter<DirSeed>,
    },
}

pub(super) enum WalkEvent {
    Node(FsWalkNode),
    Checkpoint,
}

pub(super) struct ServerWalker {
    exports: Arc<Mutex<ShareExportConfig>>,
    work: Vec<WalkWork>,
    next_id: u64,
    reserved: usize,
    name_bytes: usize,
}

impl ServerWalker {
    pub(super) fn new(root: String, exports: Arc<Mutex<ShareExportConfig>>) -> io::Result<Self> {
        let (parent_path, name) = split_root(&root);
        validate_name(&name, true)?;
        Ok(Self {
            exports,
            work: vec![WalkWork::Enter(DirSeed {
                id: 0,
                parent: None,
                depth: 1,
                name: name.clone(),
                parent_path: parent_path.into(),
            })],
            next_id: 1,
            reserved: 1,
            name_bytes: name.len(),
        })
    }

    pub(super) fn next_event(&mut self) -> io::Result<Option<WalkEvent>> {
        loop {
            let Some(work) = self.work.pop() else {
                return Ok(None);
            };
            match work {
                WalkWork::Enter(dir) => {
                    let path = dir.path();
                    let meta = fs::stat(&path, &self.exports)?;
                    if meta.is_symlink {
                        if dir.parent.is_none() {
                            return Err(eio("share walk root is a symlink/reparse point"));
                        }
                        continue;
                    }
                    if !meta.is_dir {
                        return Err(eio("share walk directory changed during scan"));
                    }
                    let entries = fs::list_dir(&path, &self.exports)?;
                    self.work.push(WalkWork::Entries {
                        dir,
                        path: path.into(),
                        entries: entries.into_iter(),
                        subdirs: Vec::new(),
                    });
                    return Ok(Some(WalkEvent::Checkpoint));
                }
                WalkWork::Entries {
                    dir,
                    path,
                    mut entries,
                    subdirs,
                } => {
                    if let Some(meta) = entries.next() {
                        self.work.push(WalkWork::Entries {
                            dir: dir.clone(),
                            path: path.clone(),
                            entries,
                            subdirs,
                        });
                        if meta.is_symlink {
                            continue;
                        }
                        validate_name(&meta.name, false)?;
                        let id = self.reserve(&meta.name)?;
                        if meta.is_dir {
                            let depth = dir
                                .depth
                                .checked_add(1)
                                .filter(|depth| *depth <= MAX_WALK_DEPTH)
                                .ok_or_else(|| eio("share walk exceeds the depth safety limit"))?;
                            let Some(WalkWork::Entries { subdirs, .. }) = self.work.last_mut()
                            else {
                                return Err(eio("share walk state is inconsistent"));
                            };
                            subdirs.push(DirSeed {
                                id,
                                parent: Some(dir.id),
                                depth,
                                parent_path: path,
                                name: meta.name,
                            });
                            continue;
                        }
                        return Ok(Some(WalkEvent::Node(FsWalkNode {
                            id,
                            parent: Some(dir.id),
                            name: meta.name,
                            is_dir: false,
                            size: meta.size,
                        })));
                    }
                    self.work.push(WalkWork::Descend {
                        dir,
                        subdirs: subdirs.into_iter(),
                    });
                }
                WalkWork::Descend { dir, mut subdirs } => {
                    if let Some(child) = subdirs.next() {
                        self.work.push(WalkWork::Descend { dir, subdirs });
                        self.work.push(WalkWork::Enter(child));
                        continue;
                    }
                    return Ok(Some(WalkEvent::Node(FsWalkNode {
                        id: dir.id,
                        parent: dir.parent,
                        name: dir.name,
                        is_dir: true,
                        size: 0,
                    })));
                }
            }
        }
    }

    fn reserve(&mut self, name: &str) -> io::Result<u64> {
        if self.reserved >= MAX_WALK_NODES {
            return Err(eio("share walk exceeds the 2,000,000-node safety limit"));
        }
        self.name_bytes = self
            .name_bytes
            .checked_add(name.len())
            .filter(|n| *n <= MAX_WALK_NAME_BYTES)
            .ok_or_else(|| eio("share walk exceeds the name-memory safety limit"))?;
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| eio("share walk node id overflow"))?;
        self.reserved += 1;
        Ok(id)
    }
}

#[derive(Default)]
pub(super) struct NodeBatch(Vec<FsWalkNode>);

impl NodeBatch {
    pub(super) fn push(&mut self, node: FsWalkNode) -> Option<Vec<FsWalkNode>> {
        self.0.push(node);
        (self.0.len() == WALK_BATCH_NODES).then(|| self.take())
    }

    fn take(&mut self) -> Vec<FsWalkNode> {
        std::mem::replace(&mut self.0, Vec::with_capacity(WALK_BATCH_NODES))
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn norm_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        "/".into()
    } else {
        trimmed.into()
    }
}

fn split_root(path: &str) -> (String, String) {
    let path = norm_path(path);
    if path == "/" {
        return (String::new(), path);
    }
    match path.rsplit_once('/') {
        Some((parent, name)) => (parent.into(), name.into()),
        None => (String::new(), path),
    }
}

fn child_path(parent: &str, name: &str) -> String {
    format!("{}/{}", parent.trim_end_matches('/'), name)
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "peer tree walk canceled")
}
