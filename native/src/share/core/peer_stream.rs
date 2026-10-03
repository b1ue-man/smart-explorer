//! Shared streaming deadlines and explicit cancellation for RV1 peer requests.
use super::{
    backend::PeerBackend,
    framing, io_deadline,
    wire::{Ctrl, FsHostFeatures, FsRequest, FsResponse},
};
use iroh::endpoint::{RecvStream, SendStream, VarInt};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(super) fn features(backend: &PeerBackend, root: &str) -> io::Result<FsHostFeatures> {
    if backend.legacy_capabilities()? {
        return Ok(FsHostFeatures::default());
    }
    let generation = backend.node.outgoing_generation(backend.initial_endpoint());
    if let Some((known, features)) = backend.transfer.features.lock().ok().and_then(|slot| *slot) {
        if generation.is_some() && known == generation {
            return Ok(features);
        }
    }
    let features = match backend.request(FsRequest::Capabilities {
        path: root.into(),
        acquire_lease: false,
        lease_request_id: None,
    })? {
        FsResponse::Capabilities { capabilities, .. } => capabilities.features,
        _ => return Err(invalid("Gegenstelle meldet keine Fähigkeiten")),
    };
    if let Ok(mut slot) = backend.transfer.features.lock() {
        *slot = Some((
            backend.node.outgoing_generation(backend.initial_endpoint()),
            features,
        ));
    }
    Ok(features)
}
pub(super) fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text)
}
pub(super) fn transport(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotConnected
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::TimedOut
            | io::ErrorKind::UnexpectedEof
    )
}
pub(super) fn abort(send: &mut SendStream, recv: &mut RecvStream, canceled: bool) {
    if canceled {
        let _ = recv.stop(VarInt::from_u32(super::analysis_tasks::CANCEL_CODE));
        let _ = send.reset(VarInt::from_u32(super::analysis_tasks::CANCEL_CODE));
    } else {
        io_deadline::abort(send, recv);
    }
}
pub(super) async fn frame(
    recv: &mut RecvStream,
    cancel: &AtomicBool,
    seconds: u64,
) -> io::Result<(u8, Vec<u8>)> {
    let next = framing::recv_tagged_limited(recv, super::fs_response::STREAM_FRAME_LIMIT);
    tokio::pin!(next);
    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result = &mut next => return result,
            _ = poll.tick() => if cancel.load(Ordering::Relaxed) { return Err(io::ErrorKind::Interrupted.into()); },
            _ = &mut deadline => return Err(io::Error::new(io::ErrorKind::TimedOut, "Keine vollständige Share-Meldung innerhalb des Zeitlimits")),
        }
    }
}
pub(super) async fn response(recv: &mut RecvStream, cancel: &AtomicBool) -> io::Result<FsResponse> {
    let (tag, bytes) = frame(recv, cancel, 60).await?;
    decode(tag, &bytes)
}
pub(super) fn decode(tag: u8, bytes: &[u8]) -> io::Result<FsResponse> {
    if tag != framing::TAG_CTRL {
        return Err(invalid("Share-Steuermeldung fehlt"));
    }
    match serde_json::from_slice(bytes).map_err(io::Error::other)? {
        Ctrl::FsResp { resp } => framing::decode_resp(resp),
        _ => Err(invalid("Unerwartete Share-Steuermeldung")),
    }
}
pub(super) fn call<T>(
    backend: &PeerBackend,
    request: FsRequest,
    cancel: &AtomicBool,
    mut message: impl FnMut(FsResponse) -> io::Result<Option<T>>,
) -> io::Result<T> {
    let endpoint = backend.current_endpoint()?;
    let mut opened = backend.node.open_stream(&endpoint, &backend.identity)?;
    let lease = backend.mount_lease_token()?;
    let result = backend.node.block_on(async {
        io_deadline::run(
            "Share streaming request",
            framing::send_ctrl(
                &mut opened.send,
                &Ctrl::Fs {
                    req: request,
                    lease,
                },
            ),
        )
        .await?;
        loop {
            if let Some(result) = message(response(&mut opened.recv, cancel).await?)? {
                return Ok(result);
            }
        }
    });
    if let Err(error) = &result {
        abort(
            &mut opened.send,
            &mut opened.recv,
            cancel.load(Ordering::Relaxed),
        );
        if transport(error) {
            let _ = backend
                .node
                .invalidate_outgoing_session(&opened.session_key, opened.generation);
        }
    }
    result
}
pub(super) fn relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains('\0'))
}
