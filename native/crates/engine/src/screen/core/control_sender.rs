//! Bounded control writes share a peer reserve, including large window/icon replies.
use super::sender_budget::{reserve_control, ControlPermit};
use super::wire::{invalid, tag_limit, MAX_CONTROL_BYTES};
use super::wire_control::{encode_host_frame, HostFrame};
use std::io;
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};

async fn permit(peer: &str, bytes: usize) -> io::Result<ControlPermit> {
    // The same bounded handshake/control timeout used by the screen transport.
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(permit) = reserve_control(peer.to_owned(), bytes) {
                return permit;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "screen control reserve unavailable",
        )
    })
}

pub async fn send<W: AsyncWrite + Unpin>(
    peer: String,
    writer: &mut W,
    frame: &HostFrame,
) -> io::Result<()> {
    send_counted(peer, writer, frame).await.map(|_| ())
}

/// The bytes `frame` takes on the control stream: tag, length, payload.
pub fn wire_bytes(frame: &HostFrame) -> u64 {
    encode_host_frame(frame).map_or(0, |(_, payload)| 5 + payload.len() as u64)
}

/// `frame` as the control stream carries it: tag, length, payload (round 8,
/// Codex 8/5: a large frame goes piece by piece, `write_piece`).
pub fn encode_wire(frame: &HostFrame) -> io::Result<Vec<u8>> {
    let (tag, payload) = encode_host_frame(frame)?;
    if payload.len() > tag_limit(tag) {
        return Err(invalid("screen control frame exceeds tag limit"));
    }
    let len =
        u32::try_from(payload.len()).map_err(|_| invalid("screen control length overflow"))?;
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(tag);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// One piece of an encoded frame (`encode_wire`), each write within the
/// peer's control reserve like `send_counted`'s.
pub async fn write_piece<W: AsyncWrite + Unpin>(
    peer: &str,
    writer: &mut W,
    piece: &[u8],
) -> io::Result<()> {
    for chunk in piece.chunks(MAX_CONTROL_BYTES) {
        let _permit = permit(peer, chunk.len()).await?;
        writer.write_all(chunk).await?;
        writer.flush().await?;
    }
    Ok(())
}

/// A frame outside `HostFrame` (`video_transport::VIDEO_ANSWER`), whole and
/// within the peer's control reserve like `send_counted`; returns its bytes.
pub async fn send_raw<W: AsyncWrite + Unpin>(
    peer: &str,
    writer: &mut W,
    tag: u8,
    payload: &[u8],
) -> io::Result<u64> {
    if payload.len() > tag_limit(tag) {
        return Err(invalid("screen control frame exceeds tag limit"));
    }
    let len =
        u32::try_from(payload.len()).map_err(|_| invalid("screen control length overflow"))?;
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(tag);
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(payload);
    write_piece(peer, writer, &frame).await?;
    Ok(frame.len() as u64)
}

/// `send`, with the bytes it wrote (review 2026-11-19: the control stream
/// counts against „Höchste Datenrate“, `Pipeline::book_control_bytes`).
pub async fn send_counted<W: AsyncWrite + Unpin>(
    peer: String,
    writer: &mut W,
    frame: &HostFrame,
) -> io::Result<u64> {
    let (tag, payload) = encode_host_frame(frame)?;
    if payload.len() > tag_limit(tag) {
        return Err(invalid("screen control frame exceeds tag limit"));
    }
    let len =
        u32::try_from(payload.len()).map_err(|_| invalid("screen control length overflow"))?;
    {
        let _permit = permit(&peer, 5).await?;
        writer.write_all(&[tag]).await?;
        writer.write_all(&len.to_be_bytes()).await?;
    }
    // Large bounded lists retain their wire contract; each actual write holds
    // only its own reserved slice, leaving room for sibling session control.
    for chunk in payload.chunks(MAX_CONTROL_BYTES) {
        let _permit = permit(&peer, chunk.len()).await?;
        writer.write_all(chunk).await?;
        writer.flush().await?;
    }
    Ok(5 + payload.len() as u64)
}
