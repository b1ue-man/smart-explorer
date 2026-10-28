use std::io;

use iroh::endpoint::{ReadExactError, RecvStream, SendStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::core::eio;
use super::wire::{Ctrl, FsResponse};

#[path = "lz4_stream.rs"]
pub(in crate::share) mod lz4_stream;

pub(super) const TAG_CTRL: u8 = 0;
pub(super) const TAG_DATA: u8 = 1;
/// A content frame whose payload is an LZ4 block (`lz4_v1`): a kind byte (0
/// raw, 1 LZ4), for 1 the raw length as a little-endian `u32`, then the
/// block. Only streams that both sides negotiated carry it, and only for file
/// content; control frames never do.
pub(super) const TAG_DATA_LZ4: u8 = 2;
const MAX_FRAME: usize = 16 * 1024 * 1024;
pub(super) const MAX_HANDSHAKE_CTRL_FRAME: usize = 64 * 1024;
pub(super) const MAX_REQUEST_CTRL_FRAME: usize = 256 * 1024;
/// A data frame carries at most one chunk (every host and client splits file
/// bytes by `fs::CHUNK`) plus its tag byte. Transfer streams read with this
/// limit, so a peer cannot make them buffer more than the memory their
/// admission reserved; closing control frames of those streams are far
/// smaller.
pub(super) const MAX_DATA_FRAME: usize = super::fs::CHUNK + 1;
/// The same for a stream that negotiated LZ4: a block is at most one chunk
/// plus its kind byte or, packed, its kind byte and raw length.
pub(super) const MAX_DATA_FRAME_LZ4: usize = super::fs::CHUNK + 1 + 5;

pub(super) async fn reply(send: &mut SendStream, resp: FsResponse) -> io::Result<()> {
    send_ctrl(send, &Ctrl::FsResp { resp }).await
}

pub(super) async fn reply_err(send: &mut SendStream, error: io::Error) -> io::Result<()> {
    reply(send, super::fs_error::response(&error)).await
}

pub(super) async fn send_ctrl(send: &mut SendStream, ctrl: &Ctrl) -> io::Result<()> {
    send_tagged(send, TAG_CTRL, &serde_json::to_vec(ctrl).map_err(eio)?).await
}

pub(super) async fn recv_ctrl(recv: &mut RecvStream) -> io::Result<Ctrl> {
    recv_ctrl_limited(recv, MAX_FRAME).await
}

pub(super) async fn recv_ctrl_limited(recv: &mut RecvStream, max_frame: usize) -> io::Result<Ctrl> {
    let (tag, payload) = recv_tagged_limited(recv, max_frame).await?;
    if tag != TAG_CTRL {
        return Err(eio("Peer sendet keinen Steuerframe"));
    }
    serde_json::from_slice::<Ctrl>(&payload).map_err(eio)
}

pub(super) async fn recv_resp(recv: &mut RecvStream) -> io::Result<FsResponse> {
    decode_resp(recv_resp_wire(recv).await?)
}

pub(super) async fn recv_resp_wire(recv: &mut RecvStream) -> io::Result<FsResponse> {
    match recv_ctrl(recv).await? {
        Ctrl::FsResp { resp } => Ok(resp),
        _ => Err(eio("Peer sendet falsche Antwort")),
    }
}

pub(super) fn decode_resp(resp: FsResponse) -> io::Result<FsResponse> {
    match resp {
        FsResponse::Err { kind, msg } => Err(super::fs_error::into_io(kind, msg)),
        response => Ok(response),
    }
}

pub(super) async fn send_tagged(send: &mut SendStream, tag: u8, payload: &[u8]) -> io::Result<()> {
    let head = frame_head(tag, payload.len())?;
    // Length, tag and payload as one write of two chunks: one lock of the
    // connection and one wake-up of its driver per frame while the window
    // has room (Z-P5). The payload is copied once, as `write` would; the
    // chunks' type is the stream's own (`Bytes` from a `Vec`, no copy).
    send.write_all_chunks(&mut [head.to_vec().into(), payload.to_vec().into()])
        .await
        .map_err(io::Error::from)?;
    send.flush().await.map_err(eio)
}

/// A frame whose payload follows `FRAME_HEAD` reserved bytes in `frame`:
/// the head is written in place and the whole frame goes out in one write
/// (the host reader's pooled buffers, Z-P5).
pub(super) async fn send_framed(
    send: &mut SendStream,
    tag: u8,
    frame: &mut [u8],
) -> io::Result<()> {
    let payload = frame
        .len()
        .checked_sub(FRAME_HEAD)
        .ok_or_else(|| eio("Frame ohne Kopf"))?;
    let head = frame_head(tag, payload)?;
    frame[..FRAME_HEAD].copy_from_slice(&head);
    send.write_all(frame).await.map_err(io::Error::from)?;
    send.flush().await.map_err(eio)
}

/// Bytes in front of a frame's payload: length (`u32`, big endian, counting
/// the tag) and tag.
pub(super) const FRAME_HEAD: usize = 5;

fn frame_head(tag: u8, payload: usize) -> io::Result<[u8; FRAME_HEAD]> {
    let n = payload
        .checked_add(1)
        .ok_or_else(|| eio("Frame zu gross"))?;
    if n > MAX_FRAME {
        return Err(eio("Frame zu gross"));
    }
    let mut head = [0u8; FRAME_HEAD];
    head[..4].copy_from_slice(&(n as u32).to_be_bytes());
    head[4] = tag;
    Ok(head)
}

/// One frame of a transfer stream: a data chunk or its closing control frame.
pub(super) async fn recv_data_frame(recv: &mut RecvStream) -> io::Result<(u8, Vec<u8>)> {
    recv_tagged_limited(recv, MAX_DATA_FRAME).await
}

/// One frame of a content stream that may carry LZ4 blocks (`lz4`).
pub(super) async fn recv_content_frame(
    recv: &mut RecvStream,
    lz4: bool,
) -> io::Result<(u8, Vec<u8>)> {
    let limit = if lz4 {
        MAX_DATA_FRAME_LZ4
    } else {
        MAX_DATA_FRAME
    };
    recv_tagged_limited(recv, limit).await
}

/// Like `recv_content_frame`, but a stream that ends cleanly between two
/// frames gives `None` (the sender of a range ends its stream after its
/// last byte).
pub(super) async fn recv_content_frame_or_end(
    recv: &mut RecvStream,
    lz4: bool,
) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; FRAME_HEAD];
    match recv.read_exact(&mut head).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(error) => return Err(read_exact_error(error)),
    }
    let limit = if lz4 {
        MAX_DATA_FRAME_LZ4
    } else {
        MAX_DATA_FRAME
    };
    let (tag, n) = parse_head(head, limit)?;
    Ok(Some((tag, read_payload(recv, n - 1).await?)))
}

pub(super) async fn recv_tagged_limited(
    recv: &mut RecvStream,
    max_frame: usize,
) -> io::Result<(u8, Vec<u8>)> {
    // Length and tag in one read; the payload is received into its final
    // buffer without zeroing it first (Z-P5).
    let mut head = [0u8; FRAME_HEAD];
    recv.read_exact(&mut head).await.map_err(read_exact_error)?;
    let (tag, n) = parse_head(head, max_frame)?;
    Ok((tag, read_payload(recv, n - 1).await?))
}

/// The tag and the validated frame length (tag included) of a head.
fn parse_head(head: [u8; FRAME_HEAD], max_frame: usize) -> io::Result<(u8, usize)> {
    let n = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    validate_frame_len(n, max_frame)?;
    Ok((head[4], n))
}

/// Exactly `len` payload bytes, read into uninitialized capacity: no
/// `memset` of a buffer that is overwritten anyway.
async fn read_payload(recv: &mut RecvStream, len: usize) -> io::Result<Vec<u8>> {
    let mut payload = Vec::with_capacity(len);
    while payload.len() < len {
        let want = (len - payload.len()) as u64;
        let read = (&mut *recv)
            .take(want)
            .read_buf(&mut payload)
            .await
            .map_err(read_io_error)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("peer stream closed after {} bytes", payload.len()),
            ));
        }
    }
    Ok(payload)
}

/// `read_exact_error` for the `AsyncRead` path: a lost connection keeps its
/// cause across the daemon error frame.
fn read_io_error(error: io::Error) -> io::Error {
    let lost = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<iroh::endpoint::ReadError>())
        .and_then(|inner| match inner {
            iroh::endpoint::ReadError::ConnectionLost(lost) => Some(lost.to_string()),
            _ => None,
        });
    match lost {
        Some(cause) => super::io_deadline::disconnected(format!("connection lost: {cause}")),
        None => error,
    }
}

fn validate_frame_len(len: usize, requested_max: usize) -> io::Result<()> {
    let max = requested_max.min(MAX_FRAME);
    if len == 0 || len > max {
        Err(eio("Frame zu gross"))
    } else {
        Ok(())
    }
}

pub(super) fn read_exact_error(error: ReadExactError) -> io::Error {
    match error {
        ReadExactError::FinishedEarly(read) => io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("peer stream closed after {read} bytes"),
        ),
        ReadExactError::ReadError(iroh::endpoint::ReadError::ConnectionLost(error)) => {
            // ReadError's Display is only "connection lost". Keep the cause
            // (idle timeout, peer close, etc.) across the daemon error frame.
            super::io_deadline::disconnected(format!("connection lost: {error}"))
        }
        ReadExactError::ReadError(error) => io::Error::from(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_open_task_quic_failure_keeps_underlying_disconnect_cause() {
        let error = read_exact_error(ReadExactError::ReadError(
            iroh::endpoint::ReadError::ConnectionLost(iroh::endpoint::ConnectionError::TimedOut),
        ));
        assert_eq!(error.kind(), io::ErrorKind::NotConnected);
        assert!(error.to_string().contains("connection lost:"));
        assert!(error.to_string().contains("timed out"));
    }

    #[test]
    fn operation_specific_frame_limits_are_stricter_than_response_budget() {
        const {
            assert!(MAX_HANDSHAKE_CTRL_FRAME < MAX_REQUEST_CTRL_FRAME);
            assert!(MAX_REQUEST_CTRL_FRAME < MAX_FRAME);
        }
        assert!(validate_frame_len(MAX_HANDSHAKE_CTRL_FRAME, MAX_HANDSHAKE_CTRL_FRAME).is_ok());
        assert!(
            validate_frame_len(MAX_HANDSHAKE_CTRL_FRAME + 1, MAX_HANDSHAKE_CTRL_FRAME).is_err()
        );
        assert!(validate_frame_len(0, MAX_REQUEST_CTRL_FRAME).is_err());
        assert!(validate_frame_len(MAX_FRAME + 1, usize::MAX).is_err());
    }

    #[test]
    fn transfer_engine_task_data_frames_hold_one_chunk() {
        assert_eq!(MAX_DATA_FRAME, super::super::fs::CHUNK + 1);
        assert!(validate_frame_len(MAX_DATA_FRAME, MAX_DATA_FRAME).is_ok());
        assert!(validate_frame_len(MAX_DATA_FRAME + 1, MAX_DATA_FRAME).is_err());
        // A write's closing frame (WriteDone with a lease) fits easily.
        let commit = serde_json::to_vec(&Ctrl::Fs {
            req: super::super::wire::FsRequest::WriteDone,
            lease: Some("L".repeat(256)),
        })
        .unwrap();
        assert!(commit.len() < MAX_DATA_FRAME);
    }
}
