use std::io;

use iroh::endpoint::SendStream;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use super::core::eio;
use super::framing::{reply, send_tagged, TAG_DATA};
use super::fs_access::FsAccess;
use super::io_deadline;
use super::walk::{ServerWalker, WalkEvent};
use super::walk_assembly::{TreeAssembler, WalkTotals, MAX_WALK_NODES};
use super::wire::FsResponse;

pub(super) const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_SNAPSHOT_NODES: u64 = MAX_WALK_NODES as u64;
const SNAPSHOT_UPDATE_NODES: u64 = 256;
const SNAPSHOT_CHANNEL: usize = 2;

pub(super) async fn serve_snapshot(
    mut send: SendStream,
    root: String,
    access: FsAccess,
) -> io::Result<()> {
    let (updates, mut received) = mpsc::channel(SNAPSHOT_CHANNEL);
    let worker = super::blocking::spawn("Share storage snapshot", move || {
        if let Err(error) = build_snapshot(root, access, updates.clone()) {
            let _ = updates.blocking_send(Err(error));
        }
        Ok(())
    })
    .await?;
    while let Some(update) = received.recv().await {
        match update {
            Err(error) => return reply_snapshot(&mut send, super::fs_error::response(&error)).await,
            Ok(update) => match update {
                SnapshotUpdate::Progress(totals) => {
                    reply_snapshot(
                        &mut send,
                        FsResponse::SnapshotProgress {
                            files: totals.files,
                            dirs: totals.dirs,
                            bytes: totals.bytes,
                            nodes: totals.nodes(),
                        },
                    )
                    .await?;
                }
                SnapshotUpdate::Ready(snapshot) => {
                    let totals = snapshot.totals;
                    reply_snapshot(
                        &mut send,
                        FsResponse::SnapshotReady {
                            encoded_len: snapshot.encoded.len() as u64,
                            sha256: sha256(&snapshot.encoded),
                            files: totals.files,
                            dirs: totals.dirs,
                            bytes: totals.bytes,
                            nodes: totals.nodes(),
                        },
                    )
                    .await?;
                    for chunk in snapshot.encoded.chunks(crate::agent_proto::CHUNK) {
                        io_deadline::run(
                            "peer storage snapshot data",
                            send_tagged(&mut send, TAG_DATA, chunk),
                        )
                        .await?;
                    }
                    reply_snapshot(
                        &mut send,
                        FsResponse::SnapshotDone {
                            files: totals.files,
                            dirs: totals.dirs,
                            bytes: totals.bytes,
                            nodes: totals.nodes(),
                        },
                    )
                    .await?;
                }
            },
        }
    }
    worker.join().await
}

fn build_snapshot(
    root: String,
    access: FsAccess,
    updates: mpsc::Sender<io::Result<SnapshotUpdate>>,
) -> io::Result<()> {
    let mut walker = ServerWalker::new(root, access)?;
    let mut tree = TreeAssembler::default();
    let mut totals = WalkTotals::default();
    loop {
        match walker.next_event()? {
            Some(WalkEvent::Node(node)) => {
                if totals.nodes() >= MAX_SNAPSHOT_NODES {
                    return Err(eio("share storage snapshot exceeds node safety limit"));
                }
                totals.observe(&node)?;
                tree.push(node)?;
                if totals.nodes() % SNAPSHOT_UPDATE_NODES == 0
                    && !send_update(&updates, SnapshotUpdate::Progress(totals))
                {
                    return Ok(());
                }
            }
            Some(WalkEvent::Checkpoint) => {
                if !send_update(&updates, SnapshotUpdate::Progress(totals)) {
                    return Ok(());
                }
            }
            None => {
                let tree = tree.finish(totals, totals.nodes())?;
                let encoded = crate::agent_proto::Frame::Tree(tree).encode(0)?;
                if encoded.len() > MAX_SNAPSHOT_BYTES {
                    return Err(eio("share storage snapshot exceeds encoded safety limit"));
                }
                let _ = send_update(&updates, SnapshotUpdate::Ready(Snapshot { encoded, totals }));
                return Ok(());
            }
        }
    }
}

fn send_update(updates: &mpsc::Sender<io::Result<SnapshotUpdate>>, update: SnapshotUpdate) -> bool {
    updates.blocking_send(Ok(update)).is_ok()
}

async fn reply_snapshot(send: &mut SendStream, response: FsResponse) -> io::Result<()> {
    io_deadline::run("peer storage snapshot response", reply(send, response)).await
}

enum SnapshotUpdate {
    Progress(WalkTotals),
    Ready(Snapshot),
}

struct Snapshot {
    encoded: Vec<u8>,
    totals: WalkTotals,
}

pub(super) fn sha256(input: &[u8]) -> [u8; 32] {
    Sha256::digest(input).into()
}
