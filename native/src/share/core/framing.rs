use std::io;

use iroh::endpoint::{ReadExactError, RecvStream, SendStream};
use tokio::io::AsyncWriteExt;

use super::core::eio;
use super::wire::{Ctrl, FsResponse};

pub(super) const TAG_CTRL: u8 = 0;
pub(super) const TAG_DATA: u8 = 1;
const MAX_FRAME: usize = 16 * 1024 * 1024;
pub(super) const MAX_HANDSHAKE_CTRL_FRAME: usize = 64 * 1024;
pub(super) const MAX_REQUEST_CTRL_FRAME: usize = 256 * 1024;
/// A data frame carries at most one chunk (every host and client splits file
/// bytes by `fs::CHUNK`) plus its tag byte. Transfer streams read with this
/// limit, so a peer cannot make them buffer more than the memory their
/// admission reserved; closing control frames of those streams are far
/// smaller.
pub(super) const MAX_DATA_FRAME: usize = super::fs::CHUNK + 1;

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
    let n = payload
        .len()
        .checked_add(1)
        .ok_or_else(|| eio("Frame zu gross"))?;
    if n > MAX_FRAME {
        return Err(eio("Frame zu gross"));
    }
    send.write_all(&(n as u32).to_be_bytes())
        .await
        .map_err(io::Error::from)?;
    send.write_all(&[tag]).await.map_err(io::Error::from)?;
    send.write_all(payload).await.map_err(io::Error::from)?;
    send.flush().await.map_err(eio)
}

/// One frame of a transfer stream: a data chunk or its closing control frame.
pub(super) async fn recv_data_frame(recv: &mut RecvStream) -> io::Result<(u8, Vec<u8>)> {
    recv_tagged_limited(recv, MAX_DATA_FRAME).await
}

pub(super) async fn recv_tagged_limited(
    recv: &mut RecvStream,
    max_frame: usize,
) -> io::Result<(u8, Vec<u8>)> {
    let mut len4 = [0u8; 4];
    recv.read_exact(&mut len4).await.map_err(read_exact_error)?;
    let n = u32::from_be_bytes(len4) as usize;
    validate_frame_len(n, max_frame)?;
    // The tag is read on its own, so the payload is received into its final
    // buffer without a second copy.
    let mut tag = [0u8; 1];
    recv.read_exact(&mut tag).await.map_err(read_exact_error)?;
    let mut payload = vec![0u8; n - 1];
    recv.read_exact(&mut payload)
        .await
        .map_err(read_exact_error)?;
    Ok((tag[0], payload))
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
