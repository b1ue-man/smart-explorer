use std::io;
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream};

use super::backend::PeerBackend;
use super::core::eio;
use super::framing::{decode_resp, recv_resp_wire, recv_tagged_limited, send_ctrl, TAG_DATA};
use super::io_deadline;
use super::storage_snapshot::{sha256, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_NODES};
use super::walk_assembly::{
    invalid, validate_name, WalkTotals, MAX_WALK_DEPTH, MAX_WALK_NAME_BYTES,
};
use super::wire::{Ctrl, FsRequest, FsResponse};

const CANCEL_POLL: Duration = Duration::from_millis(250);
const SNAPSHOT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_SNAPSHOT_DATA_FRAME: usize = crate::agent_proto::CHUNK + 1;

pub(super) fn walk_peer(
    backend: &PeerBackend,
    root: &str,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> io::Result<Option<crate::agent_proto::WireNode>> {
    let supported = match backend.request(FsRequest::Capabilities {
        path: root.to_string(),
        acquire_lease: false,
        lease_request_id: None,
    })? {
        FsResponse::Capabilities {
            storage_snapshot_v1,
            ..
        } => storage_snapshot_v1,
        _ => {
            return Err(eio(
                "unexpected response to storage snapshot capability probe",
            ))
        }
    };
    if !supported {
        return super::peer_walk::walk_peer(backend, root, on_progress);
    }
    if !on_progress(0, 0) {
        return Err(interrupted());
    }

    let endpoint = backend.current_endpoint()?;
    let mut opened = backend.node.open_stream(&endpoint, &backend.identity)?;
    let lease = backend.mount_lease_token()?;
    let result = backend.node.block_on(receive_snapshot(
        &mut opened.send,
        &mut opened.recv,
        root,
        lease,
        on_progress,
    ));
    if matches!(&result, Err(failure) if failure.invalidate_session) {
        let _ = backend
            .node
            .invalidate_outgoing_session(&opened.session_key, opened.generation);
    }
    result.map(Some).map_err(|failure| failure.error)
}

async fn receive_snapshot(
    send: &mut SendStream,
    recv: &mut RecvStream,
    root: &str,
    lease: Option<String>,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> Result<crate::agent_proto::WireNode, PeerSnapshotFailure> {
    io_deadline::run(
        "peer storage snapshot request",
        send_ctrl(
            send,
            &Ctrl::Fs {
                req: FsRequest::StorageSnapshot {
                    path: root.to_string(),
                },
                lease,
            },
        ),
    )
    .await
    .map_err(PeerSnapshotFailure::transport)?;
    let result = receive_snapshot_responses(recv, on_progress).await;
    if result.is_err() {
        io_deadline::abort(send, recv);
    }
    result
}

async fn receive_snapshot_responses(
    recv: &mut RecvStream,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> Result<crate::agent_proto::WireNode, PeerSnapshotFailure> {
    let mut progress = WalkTotals::default();
    let announcement = loop {
        match recv_control(recv, progress, on_progress).await? {
            FsResponse::SnapshotProgress {
                files,
                dirs,
                bytes,
                nodes,
            } => {
                let next = reported_totals(files, dirs, bytes, nodes)
                    .map_err(PeerSnapshotFailure::protocol)?;
                require_monotonic(progress, next).map_err(PeerSnapshotFailure::protocol)?;
                progress = next;
                if !on_progress(files, bytes) {
                    return Err(PeerSnapshotFailure::local(interrupted()));
                }
            }
            FsResponse::SnapshotReady {
                encoded_len,
                sha256,
                files,
                dirs,
                bytes,
                nodes,
            } => {
                let length = usize::try_from(encoded_len).map_err(|_| {
                    PeerSnapshotFailure::protocol(invalid(
                        "peer storage snapshot length overflows platform size",
                    ))
                })?;
                if length == 0 || length > MAX_SNAPSHOT_BYTES {
                    return Err(PeerSnapshotFailure::protocol(invalid(
                        "peer storage snapshot exceeds encoded safety limit",
                    )));
                }
                let totals = reported_totals(files, dirs, bytes, nodes)
                    .map_err(PeerSnapshotFailure::protocol)?;
                if nodes == 0 {
                    return Err(PeerSnapshotFailure::protocol(invalid(
                        "peer storage snapshot root is missing",
                    )));
                }
                require_monotonic(progress, totals).map_err(PeerSnapshotFailure::protocol)?;
                break Announcement {
                    length,
                    sha256,
                    totals,
                    nodes,
                };
            }
            _ => {
                return Err(PeerSnapshotFailure::protocol(invalid(
                    "unexpected response in peer storage snapshot",
                )))
            }
        }
    };

    let mut encoded = Vec::new();
    while encoded.len() < announcement.length {
        let (tag, chunk) = recv_data(recv, announcement.totals, on_progress).await?;
        if tag != TAG_DATA || chunk.is_empty() || chunk.len() > crate::agent_proto::CHUNK {
            return Err(PeerSnapshotFailure::protocol(invalid(
                "invalid peer storage snapshot data chunk",
            )));
        }
        let remaining = announcement.length - encoded.len();
        if chunk.len() > remaining {
            return Err(PeerSnapshotFailure::protocol(invalid(
                "peer storage snapshot exceeds announced length",
            )));
        }
        encoded
            .try_reserve(chunk.len())
            .map_err(|_| PeerSnapshotFailure::local(eio("cannot allocate snapshot buffer")))?;
        encoded.extend_from_slice(&chunk);
    }
    if sha256(&encoded) != announcement.sha256 {
        return Err(PeerSnapshotFailure::protocol(invalid(
            "peer storage snapshot SHA-256 mismatch",
        )));
    }

    match recv_control(recv, announcement.totals, on_progress).await? {
        FsResponse::SnapshotDone {
            files,
            dirs,
            bytes,
            nodes,
        } => {
            let totals = reported_totals(files, dirs, bytes, nodes)
                .map_err(PeerSnapshotFailure::protocol)?;
            if totals != announcement.totals || nodes != announcement.nodes {
                return Err(PeerSnapshotFailure::protocol(invalid(
                    "peer storage snapshot completion totals mismatch",
                )));
            }
        }
        _ => {
            return Err(PeerSnapshotFailure::protocol(invalid(
                "peer storage snapshot missing completion",
            )))
        }
    }

    let (request_id, frame) =
        crate::agent_proto::Frame::decode(&encoded).map_err(PeerSnapshotFailure::protocol)?;
    if request_id != 0 {
        return Err(PeerSnapshotFailure::protocol(invalid(
            "peer storage snapshot has an unexpected frame id",
        )));
    }
    let crate::agent_proto::Frame::Tree(tree) = frame else {
        return Err(PeerSnapshotFailure::protocol(invalid(
            "peer storage snapshot does not contain a tree frame",
        )));
    };
    let totals = validate_tree(&tree).map_err(PeerSnapshotFailure::protocol)?;
    if totals != announcement.totals || totals.nodes() != announcement.nodes {
        return Err(PeerSnapshotFailure::protocol(invalid(
            "peer storage snapshot tree totals mismatch",
        )));
    }
    if !on_progress(totals.files, totals.bytes) {
        return Err(PeerSnapshotFailure::local(interrupted()));
    }
    Ok(tree)
}

async fn recv_control(
    recv: &mut RecvStream,
    progress: WalkTotals,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> Result<FsResponse, PeerSnapshotFailure> {
    let response = recv_resp_wire(recv);
    tokio::pin!(response);
    let idle = tokio::time::sleep(SNAPSHOT_IDLE_TIMEOUT);
    tokio::pin!(idle);
    let mut poll = tokio::time::interval(CANCEL_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    poll.tick().await;
    loop {
        tokio::select! {
            biased;
            response = &mut response => {
                let response = response.map_err(PeerSnapshotFailure::transport)?;
                return decode_resp(response).map_err(PeerSnapshotFailure::remote);
            }
            _ = poll.tick() => {
                if !on_progress(progress.files, progress.bytes) {
                    return Err(PeerSnapshotFailure::local(interrupted()));
                }
            }
            _ = &mut idle => return Err(PeerSnapshotFailure::transport(io::Error::new(
                io::ErrorKind::TimedOut,
                "peer storage snapshot made no progress for 60 seconds",
            ))),
        }
    }
}

async fn recv_data(
    recv: &mut RecvStream,
    progress: WalkTotals,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> Result<(u8, Vec<u8>), PeerSnapshotFailure> {
    let data = recv_tagged_limited(recv, MAX_SNAPSHOT_DATA_FRAME);
    tokio::pin!(data);
    let idle = tokio::time::sleep(SNAPSHOT_IDLE_TIMEOUT);
    tokio::pin!(idle);
    let mut poll = tokio::time::interval(CANCEL_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    poll.tick().await;
    loop {
        tokio::select! {
            biased;
            data = &mut data => return data.map_err(PeerSnapshotFailure::transport),
            _ = poll.tick() => {
                if !on_progress(progress.files, progress.bytes) {
                    return Err(PeerSnapshotFailure::local(interrupted()));
                }
            }
            _ = &mut idle => return Err(PeerSnapshotFailure::transport(io::Error::new(
                io::ErrorKind::TimedOut,
                "peer storage snapshot data stalled for 60 seconds",
            ))),
        }
    }
}

fn reported_totals(files: u64, dirs: u64, bytes: u64, nodes: u64) -> io::Result<WalkTotals> {
    let counted = files
        .checked_add(dirs)
        .ok_or_else(|| invalid("peer storage snapshot node totals overflow"))?;
    if counted != nodes || nodes > MAX_SNAPSHOT_NODES {
        return Err(invalid(
            "peer storage snapshot node totals exceed or contradict the safety limit",
        ));
    }
    Ok(WalkTotals { files, dirs, bytes })
}

fn require_monotonic(previous: WalkTotals, next: WalkTotals) -> io::Result<()> {
    if next.files < previous.files || next.dirs < previous.dirs || next.bytes < previous.bytes {
        return Err(invalid("peer storage snapshot progress moved backwards"));
    }
    Ok(())
}

fn validate_tree(tree: &crate::agent_proto::WireNode) -> io::Result<WalkTotals> {
    fn visit(
        node: &crate::agent_proto::WireNode,
        depth: usize,
        root: bool,
        names: &mut usize,
        nodes: &mut u64,
    ) -> io::Result<WalkTotals> {
        if depth > MAX_WALK_DEPTH {
            return Err(invalid("peer storage snapshot exceeds depth safety limit"));
        }
        validate_name(&node.name, root)?;
        if root && !node.is_dir {
            return Err(invalid("peer storage snapshot root is not a directory"));
        }
        *nodes = nodes
            .checked_add(1)
            .filter(|nodes| *nodes <= MAX_SNAPSHOT_NODES)
            .ok_or_else(|| invalid("peer storage snapshot has too many nodes"))?;
        *names = names
            .checked_add(node.name.len())
            .filter(|names| *names <= MAX_WALK_NAME_BYTES)
            .ok_or_else(|| invalid("peer storage snapshot name data exceeds safety limit"))?;

        let mut totals = WalkTotals::default();
        let mut child_size = 0u64;
        for child in &node.children {
            let child_totals = visit(child, depth + 1, false, names, nodes)?;
            child_size = child_size
                .checked_add(child.size)
                .ok_or_else(|| invalid("peer storage snapshot size overflow"))?;
            totals.files = totals
                .files
                .checked_add(child_totals.files)
                .ok_or_else(|| invalid("peer storage snapshot totals overflow"))?;
            totals.dirs = totals
                .dirs
                .checked_add(child_totals.dirs)
                .ok_or_else(|| invalid("peer storage snapshot totals overflow"))?;
            totals.bytes = totals
                .bytes
                .checked_add(child_totals.bytes)
                .ok_or_else(|| invalid("peer storage snapshot totals overflow"))?;
        }
        if node.is_dir {
            if node.size != child_size {
                return Err(invalid("peer storage snapshot directory size mismatch"));
            }
            totals.dirs = totals
                .dirs
                .checked_add(1)
                .ok_or_else(|| invalid("peer storage snapshot totals overflow"))?;
        } else {
            if !node.children.is_empty() {
                return Err(invalid("peer storage snapshot file has children"));
            }
            totals.files = totals
                .files
                .checked_add(1)
                .ok_or_else(|| invalid("peer storage snapshot totals overflow"))?;
            totals.bytes = totals
                .bytes
                .checked_add(node.size)
                .ok_or_else(|| invalid("peer storage snapshot totals overflow"))?;
        }
        Ok(totals)
    }

    let mut names = 0;
    let mut nodes = 0;
    visit(tree, 1, true, &mut names, &mut nodes)
}

struct Announcement {
    length: usize,
    sha256: [u8; 32],
    totals: WalkTotals,
    nodes: u64,
}

struct PeerSnapshotFailure {
    error: io::Error,
    invalidate_session: bool,
}

impl PeerSnapshotFailure {
    fn transport(error: io::Error) -> Self {
        Self {
            error,
            invalidate_session: true,
        }
    }

    fn protocol(error: io::Error) -> Self {
        Self::transport(error)
    }

    fn remote(error: io::Error) -> Self {
        Self {
            error,
            invalidate_session: false,
        }
    }

    fn local(error: io::Error) -> Self {
        Self::remote(error)
    }
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "peer storage snapshot canceled")
}

#[cfg(test)]
#[path = "peer_storage_snapshot_task_tests.rs"]
mod share_remote_task_tests;
