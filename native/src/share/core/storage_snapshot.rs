//! Legacy snapshots use the same tolerant host worker, within the old receiver's encoding limits.
use super::{
    analysis_admission,
    framing::{reply, send_tagged, TAG_DATA},
    fs_access::FsAccess,
    host_stream::Stop,
    io_deadline,
    session::PeerPrincipal,
    walk_assembly::{WalkTotals, MAX_WALK_NODES},
    wire::FsResponse,
};
use crate::analytics::{Progress, ScanStatus, SizeNode};
use iroh::endpoint::SendStream;
use sha2::{Digest, Sha256};
use std::{io, time::Duration};

pub(super) const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_SNAPSHOT_NODES: u64 = MAX_WALK_NODES as u64;

pub(super) async fn serve_snapshot(
    mut send: SendStream,
    root: String,
    access: FsAccess,
    principal: PeerPrincipal,
) -> io::Result<()> {
    let p = Progress::default();
    let _stop = Stop(p.cancel.clone());
    access.register_cancel(&p.cancel)?;
    let authority = access.clone();
    let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::PeerService);
    let stopped = send.stopped();
    tokio::pin!(stopped);
    let ticket = analysis_admission::host().enqueue(principal);
    let acquire = ticket.acquire();
    tokio::pin!(acquire);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let permit = loop {
        tokio::select! {
            permit=&mut acquire=>break permit,
            _=&mut stopped=>return Err(io::ErrorKind::Interrupted.into()),
            _=tick.tick()=>{ authority.check_read()?; heartbeat(&mut send).await?; },
        }
    };
    let (done, mut received) = tokio::sync::oneshot::channel();
    let worker = p.clone();
    std::thread::Builder::new()
        .name("share-legacy-analysis".into())
        .stack_size(crate::analytics::SCAN_THREAD_STACK_BYTES)
        .spawn(move || {
            let _permit = permit;
            let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::PeerService);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                build(&root, &access, &worker)
            }))
            .unwrap_or_else(|_| Err(io::Error::other("Legacy-Analyse-Worker unerwartet beendet")));
            let _ = done.send(result);
        })?;
    let snapshot = loop {
        tokio::select! {
            _=&mut stopped=>return Err(io::ErrorKind::Interrupted.into()),
            _=tick.tick()=>{ authority.check_read()?; heartbeat(&mut send).await?; },
            result=&mut received=>match result.map_err(io::Error::other)? {
                Ok(snapshot)=>break snapshot,Err(error)=>return super::framing::reply_err(&mut send,error).await,
            }
        }
    };
    let totals = snapshot.totals;
    authority.check_read()?;
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
        authority.check_read()?;
        tokio::select! {
            _=&mut stopped=>return Err(io::ErrorKind::Interrupted.into()),
            result=io_deadline::run("legacy snapshot data",send_tagged(&mut send,TAG_DATA,chunk))=>result?,
        }
    }
    authority.check_read()?;
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
    send.finish().map_err(io::Error::other)
}
fn build(root: &str, access: &FsAccess, p: &Progress) -> io::Result<Snapshot> {
    let nodes = (crate::transfer::memory_budget() as u64 / 128)
        .clamp(2, MAX_SNAPSHOT_NODES)
        .min(MAX_SNAPSHOT_BYTES as u64 / 128);
    let mut result =
        crate::share::storage_analysis_host::scan_bounded(root, access, p, Some(nodes));
    if result.status == ScanStatus::Canceled {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let mut tree = result.tree.take().ok_or_else(|| {
        io::Error::other(
            result
                .issues
                .first()
                .map_or("Analyse-Wurzel nicht lesbar", |i| i.detail.as_str()),
        )
    })?;
    legacy_depth(&mut tree, 1);
    // The legacy name encoding may have more overhead than the v2 shape:
    // compact the existing result further when the actual bytes require it.
    let mut limit = nodes;
    loop {
        p.check_cancel()?;
        crate::analytics::fit_tree(&mut tree, limit, p)?;
        let wire = wire_tree(&tree, p)?;
        let totals = count(&wire)?;
        let encoded = crate::agent_proto::Frame::Tree(wire).encode(0)?;
        if encoded.len() <= MAX_SNAPSHOT_BYTES {
            return Ok(Snapshot { encoded, totals });
        }
        if limit <= 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Legacy-Ergebnis passt nicht in das vereinbarte Format",
            ));
        }
        limit = (limit / 2).max(2);
    }
}
fn legacy_depth(node: &mut SizeNode, depth: usize) {
    if depth >= super::walk_assembly::MAX_WALK_DEPTH - 1 && !node.children.is_empty() {
        node.children = vec![SizeNode {
            name: crate::analytics::aggregate_name(1).into(),
            size: node.size,
            is_dir: false,
            children: Vec::new(),
        }];
    } else {
        for child in &mut node.children {
            legacy_depth(child, depth + 1);
        }
    }
}
fn wire_tree(tree: &SizeNode, p: &Progress) -> io::Result<crate::agent_proto::WireNode> {
    p.check_cancel()?;
    Ok(crate::agent_proto::WireNode {
        name: tree.name.to_string(),
        size: tree.size,
        is_dir: tree.is_dir,
        children: tree
            .children
            .iter()
            .map(|child| wire_tree(child, p))
            .collect::<io::Result<_>>()?,
    })
}
fn count(root: &crate::agent_proto::WireNode) -> io::Result<WalkTotals> {
    let mut totals = WalkTotals::default();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_dir {
            totals.dirs = totals
                .dirs
                .checked_add(1)
                .ok_or_else(|| io::Error::other("Zählerüberlauf"))?;
        } else {
            totals.files = totals
                .files
                .checked_add(1)
                .ok_or_else(|| io::Error::other("Zählerüberlauf"))?;
        }
        stack.extend(node.children.iter());
    }
    totals.bytes = root.size;
    Ok(totals)
}
async fn heartbeat(send: &mut SendStream) -> io::Result<()> {
    // Old receivers equate counts with retained nodes and demand monotonicity;
    // zero keeps that contract while the tolerant scanner aggregates detail.
    reply_snapshot(
        send,
        FsResponse::SnapshotProgress {
            files: 0,
            dirs: 0,
            bytes: 0,
            nodes: 0,
        },
    )
    .await
}
async fn reply_snapshot(send: &mut SendStream, response: FsResponse) -> io::Result<()> {
    io_deadline::run("legacy snapshot response", reply(send, response)).await
}
struct Snapshot {
    encoded: Vec<u8>,
    totals: WalkTotals,
}
pub(super) fn sha256(input: &[u8]) -> [u8; 32] {
    Sha256::digest(input).into()
}
