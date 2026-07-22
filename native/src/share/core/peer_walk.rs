use std::io;
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream};

use super::backend::{send_ctrl, PeerBackend};
use super::framing::{decode_resp, recv_resp_wire};
use super::io_deadline;
use super::walk_assembly::{invalid, TreeAssembler, WalkTotals};
use super::wire::{Ctrl, FsRequest, FsResponse};

const CANCEL_POLL: Duration = Duration::from_millis(250);
const WALK_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) fn walk_peer(
    backend: &PeerBackend,
    root: &str,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> io::Result<Option<crate::agent_proto::WireNode>> {
    if !on_progress(0, 0) {
        return Err(interrupted());
    }
    let endpoint = backend.current_endpoint()?;
    let mut opened = backend.node.open_stream(&endpoint, &backend.identity)?;
    let lease = backend.mount_lease_token()?;
    let result = backend.node.block_on(receive_walk(
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

async fn receive_walk(
    send: &mut SendStream,
    recv: &mut RecvStream,
    root: &str,
    lease: Option<String>,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> Result<crate::agent_proto::WireNode, PeerWalkFailure> {
    io_deadline::run(
        "peer tree request",
        send_ctrl(
            send,
            &Ctrl::Fs {
                req: FsRequest::WalkTree {
                    path: root.to_string(),
                },
                lease,
            },
        ),
    )
    .await
    .map_err(PeerWalkFailure::transport)?;
    let result = receive_responses(recv, on_progress).await;
    if result.is_err() {
        io_deadline::abort(send, recv);
    }
    result
}

async fn receive_responses(
    recv: &mut RecvStream,
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> Result<crate::agent_proto::WireNode, PeerWalkFailure> {
    let mut tree = TreeAssembler::default();
    let mut progress = (0u64, 0u64);
    loop {
        match recv_checked(recv, progress, on_progress).await? {
            FsResponse::WalkBatch {
                nodes,
                files,
                dirs,
                bytes,
            } => {
                tree.push_batch(nodes, WalkTotals { files, dirs, bytes })
                    .map_err(PeerWalkFailure::protocol)?;
                progress = (files, bytes);
                if !on_progress(files, bytes) {
                    return Err(PeerWalkFailure::local(interrupted()));
                }
            }
            FsResponse::WalkDone {
                files,
                dirs,
                bytes,
                nodes,
            } => {
                if !on_progress(files, bytes) {
                    return Err(PeerWalkFailure::local(interrupted()));
                }
                return tree
                    .finish(WalkTotals { files, dirs, bytes }, nodes)
                    .map_err(PeerWalkFailure::protocol);
            }
            _ => {
                return Err(PeerWalkFailure::protocol(invalid(
                    "unexpected response in peer tree walk",
                )))
            }
        }
    }
}

async fn recv_checked(
    recv: &mut RecvStream,
    progress: (u64, u64),
    on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
) -> Result<FsResponse, PeerWalkFailure> {
    let response = recv_resp_wire(recv);
    tokio::pin!(response);
    let idle = tokio::time::sleep(WALK_IDLE_TIMEOUT);
    tokio::pin!(idle);
    let mut poll = tokio::time::interval(CANCEL_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    poll.tick().await;
    loop {
        tokio::select! {
            biased;
            response = &mut response => {
                let response = response.map_err(PeerWalkFailure::transport)?;
                return decode_resp(response).map_err(PeerWalkFailure::remote);
            },
            _ = poll.tick() => {
                if !on_progress(progress.0, progress.1) {
                    return Err(PeerWalkFailure::local(interrupted()));
                }
            }
            _ = &mut idle => return Err(PeerWalkFailure::transport(io::Error::new(
                io::ErrorKind::TimedOut,
                "peer tree walk made no progress for 60 seconds",
            ))),
        }
    }
}

struct PeerWalkFailure {
    error: io::Error,
    invalidate_session: bool,
}

impl PeerWalkFailure {
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
    io::Error::new(io::ErrorKind::Interrupted, "peer tree walk canceled")
}
