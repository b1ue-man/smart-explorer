use crate::analytics::{analysis_transfer::AnalysisReceiver, Progress, ScanOutcome, ScanPhase};
use iroh::endpoint::{RecvStream, SendStream};
use std::{io, time::Duration};

use super::backend::PeerBackend;
use super::framing::{self, TAG_CTRL, TAG_DATA};
use super::io_deadline;
use super::wire::{Ctrl, FsRequest, FsResponse, FsStorageAnalysis};

pub(super) fn scan(
    backend: &PeerBackend,
    root: &str,
    progress: &Progress,
) -> io::Result<Option<ScanOutcome>> {
    progress.check_cancel()?;
    let response = backend.request(FsRequest::Capabilities {
        path: root.into(),
        acquire_lease: false,
        lease_request_id: None,
    })?;
    match response {
        FsResponse::Capabilities {
            storage_analysis_v2: true,
            ..
        } => {}
        FsResponse::Capabilities { .. } => {
            progress.set_phase(ScanPhase::Legacy, root);
            return Ok(None);
        }
        _ => return Err(invalid("Gegenstelle meldet keine Analyse-Fähigkeiten")),
    }
    progress.check_cancel()?;
    let endpoint = backend.current_endpoint()?;
    let mut opened = backend.node.open_stream(&endpoint, &backend.identity)?;
    let lease = backend.mount_lease_token()?;
    let result = backend.node.block_on(receive(
        &mut opened.send,
        &mut opened.recv,
        root,
        lease,
        progress,
    ));
    if let Err(error) = &result {
        if !matches!(
            error.kind(),
            io::ErrorKind::Interrupted | io::ErrorKind::PermissionDenied
        ) {
            let _ = backend
                .node
                .invalidate_outgoing_session(&opened.session_key, opened.generation);
        }
    }
    result.map(Some)
}

async fn receive(
    send: &mut SendStream,
    recv: &mut RecvStream,
    root: &str,
    lease: Option<String>,
    progress: &Progress,
) -> io::Result<ScanOutcome> {
    let result = async {
        io_deadline::run(
            "analysis request",
            framing::send_ctrl(
                send,
                &Ctrl::Fs {
                    req: FsRequest::StorageAnalysis(FsStorageAnalysis {
                        path: root.into(),
                        ..Default::default()
                    }),
                    lease,
                },
            ),
        )
        .await?;
        receive_result(recv, progress).await
    }
    .await;
    if result.is_err() {
        io_deadline::abort(send, recv);
    }
    result
}

async fn receive_result(recv: &mut RecvStream, progress: &Progress) -> io::Result<ScanOutcome> {
    let mut receiver = AnalysisReceiver::default();
    loop {
        let (tag, bytes) = next_frame(recv, progress).await?;
        if tag == TAG_DATA {
            receiver.data(&bytes, progress)?;
        } else {
            let FsResponse::Analysis { message } = control(tag, &bytes)? else {
                return Err(invalid("Unerwartete Analyse-Meldung"));
            };
            if let Some(outcome) = receiver.control(message, progress)? {
                return Ok(outcome);
            }
        }
    }
}

async fn next_frame(recv: &mut RecvStream, progress: &Progress) -> io::Result<(u8, Vec<u8>)> {
    // One pinned read survives cancellation polls; partial frames are retained.
    let next = framing::recv_tagged_limited(recv, 1024 * 1024);
    tokio::pin!(next);
    let timeout = tokio::time::sleep(Duration::from_secs(60));
    tokio::pin!(timeout);
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result = &mut next => return result,
            _ = poll.tick() => progress.check_cancel()?,
            _ = &mut timeout => return Err(io::Error::new(io::ErrorKind::TimedOut,
                "Seit 60 Sekunden keine vollständige Analyse-Meldung der Gegenstelle")),
        }
    }
}

fn control(tag: u8, bytes: &[u8]) -> io::Result<FsResponse> {
    if tag != TAG_CTRL {
        return Err(invalid("Analyse-Steuermeldung fehlt"));
    }
    let Ctrl::FsResp { resp } = serde_json::from_slice(bytes).map_err(io::Error::other)? else {
        return Err(invalid("Unerwartete Analyse-Steuermeldung"));
    };
    framing::decode_resp(resp)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
