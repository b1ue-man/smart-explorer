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

pub(crate) async fn send<W: AsyncWrite + Unpin>(
    peer: String,
    writer: &mut W,
    frame: &HostFrame,
) -> io::Result<()> {
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
    Ok(())
}
