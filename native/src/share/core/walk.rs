use std::io;
use std::sync::Arc;
use std::time::Instant;

use iroh::endpoint::SendStream;
use tokio::sync::mpsc;

use super::backend::reply;
use super::core::eio;
use super::fs_access::FsAccess;
use super::io_deadline;
use super::walk_assembly::{
    validate_name, WalkTotals, MAX_WALK_DEPTH, MAX_WALK_NAME_BYTES, MAX_WALK_NODES,
    WALK_BATCH_NODES,
};
use super::wire::{FsResponse, FsWalkNode};

const WALK_RESPONSE_BUFFER: usize = 2;

/// Serve a walk through the stream's selected filesystem access. A mount lease
/// retains one backend/root; stateless browsing resolves current exports.
pub(super) async fn serve_walk(
    mut send: SendStream,
    root: String,
    access: FsAccess,
) -> io::Result<()> {
    let (responses, mut response_rx) = mpsc::channel(WALK_RESPONSE_BUFFER);
    let worker = super::blocking::spawn("Share tree walk", move || {
        walk_worker(root, access, responses)
    })
    .await?;
    while let Some(response) = response_rx.recv().await {
        match response {
            Ok(response) => reply_walk(&mut send, response).await?,
            Err(error) => {
                return reply_walk(&mut send, super::fs_error::response(&error)).await;
            }
        }
    }
    worker.join().await
}

fn walk_worker(
    root: String,
    access: FsAccess,
    responses: mpsc::Sender<io::Result<FsResponse>>,
) -> io::Result<()> {
    let mut walker = match ServerWalker::new(root, access) {
        Ok(walker) => walker,
        Err(error) => {
            let _ = responses.blocking_send(Err(error));
            return Ok(());
        }
    };
    let mut batch = NodeBatch::default();
    let mut totals = WalkTotals::default();
    let mut last_send = Instant::now();

    loop {
        let event = match walker.next_event() {
            Ok(event) => event,
            Err(error) => {
                let _ = responses.blocking_send(Err(error));
                return Ok(());
            }
        };
        match event {
            Some(WalkEvent::Node(node)) => {
                if let Err(error) = totals.observe(&node) {
                    let _ = responses.blocking_send(Err(error));
                    return Ok(());
                }
                if let Some(nodes) = batch.push(node) {
                    if !send_batch(&responses, nodes, totals) {
                        return Ok(());
                    }
                    last_send = Instant::now();
                }
            }
            Some(WalkEvent::Checkpoint) if last_send.elapsed() >= CANCEL_POLL => {
                if !send_batch(&responses, batch.take(), totals) {
                    return Ok(());
                }
                last_send = Instant::now();
            }
            Some(WalkEvent::Checkpoint) => {}
            None => {
                if !batch.is_empty() && !send_batch(&responses, batch.take(), totals) {
                    return Ok(());
                }
                let _ = responses.blocking_send(Ok(FsResponse::WalkDone {
                    files: totals.files,
                    dirs: totals.dirs,
                    bytes: totals.bytes,
                    nodes: totals.nodes(),
                }));
                return Ok(());
            }
        }
    }
}

fn send_batch(
    responses: &mpsc::Sender<io::Result<FsResponse>>,
    nodes: Vec<FsWalkNode>,
    totals: WalkTotals,
) -> bool {
    responses
        .blocking_send(Ok(FsResponse::WalkBatch {
            nodes,
            files: totals.files,
            dirs: totals.dirs,
            bytes: totals.bytes,
        }))
        .is_ok()
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
    access: FsAccess,
    work: Vec<WalkWork>,
    next_id: u64,
    reserved: usize,
    name_bytes: usize,
}

impl ServerWalker {
    pub(super) fn new(root: String, access: impl Into<FsAccess>) -> io::Result<Self> {
        let (parent_path, name) = split_root(&root);
        validate_name(&name, true)?;
        Ok(Self {
            access: access.into(),
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
                    let meta = self.access.stat(&path)?;
                    if meta.is_symlink {
                        if dir.parent.is_none() {
                            return Err(eio("share walk root is a symlink/reparse point"));
                        }
                        continue;
                    }
                    if !meta.is_dir {
                        return Err(eio("share walk directory changed during scan"));
                    }
                    let entries = self.access.list_dir(&path)?;
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
