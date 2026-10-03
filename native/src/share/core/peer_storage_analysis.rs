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
    if backend.legacy_capabilities()? {
        progress.set_phase(ScanPhase::Legacy, root);
        return Ok(None);
    }
    let response = backend.request(FsRequest::Capabilities {
        path: root.into(),
        acquire_lease: false,
        lease_request_id: None,
    })?;
    let features = match response {
        FsResponse::Capabilities {
            storage_analysis_v2: true,
            capabilities,
            ..
        } => capabilities.features,
        FsResponse::Capabilities { .. } => {
            progress.set_phase(ScanPhase::Legacy, root);
            return Ok(None);
        }
        _ => return Err(invalid("Gegenstelle meldet keine Analyse-Fähigkeiten")),
    };
    let request = FsStorageAnalysis {
        path: root.into(),
        node_budget: Some(progress.node_budget()),
        compress: features.analysis_deflate_v1,
        request_id: if features.analysis_reattach_v1 {
            Some(super::core::random_token(16).map_err(io::Error::other)?)
        } else {
            None
        },
    };
    for attempt in 0..2 {
        progress.check_cancel()?;
        let endpoint = backend.current_endpoint()?;
        let mut opened = match backend.node.open_stream(&endpoint, &backend.identity) {
            Ok(opened) => opened,
            Err(error)
                if attempt == 0
                    && features.analysis_reattach_v1
                    && super::peer_stream::transport(&error) =>
            {
                continue
            }
            Err(error) => return Err(error),
        };
        let lease = backend.mount_lease_token()?;
        let result = backend.node.block_on(receive(
            &mut opened.send,
            &mut opened.recv,
            request.clone(),
            lease,
            progress,
        ));
        match result {
            Ok(result) => return Ok(Some(result)),
            Err(error) => {
                if super::peer_stream::transport(&error) {
                    let _ = backend
                        .node
                        .invalidate_outgoing_session(&opened.session_key, opened.generation);
                }
                if attempt == 0
                    && features.analysis_reattach_v1
                    && super::peer_stream::transport(&error)
                {
                    progress.restart_remote();
                    continue;
                }
                return Err(error);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotConnected,
        "Analyse-Verbindung nicht wiederhergestellt",
    ))
}

async fn receive(
    send: &mut SendStream,
    recv: &mut RecvStream,
    request: FsStorageAnalysis,
    lease: Option<String>,
    progress: &Progress,
) -> io::Result<ScanOutcome> {
    let result = async {
        io_deadline::run(
            "analysis request",
            framing::send_ctrl(
                send,
                &Ctrl::Fs {
                    req: FsRequest::StorageAnalysis(request),
                    lease,
                },
            ),
        )
        .await?;
        receive_result(recv, progress).await
    }
    .await;
    if result.is_err() {
        super::peer_stream::abort(send, recv, progress.check_cancel().is_err());
    }
    result
}

async fn receive_result(recv: &mut RecvStream, progress: &Progress) -> io::Result<ScanOutcome> {
    let mut receiver = AnalysisReceiver::with_node_budget(progress.node_budget());
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
