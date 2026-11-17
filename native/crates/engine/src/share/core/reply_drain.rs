//! Waiting for the peer to acknowledge a finished reply (incoming peer
//! streams, `server.rs`).
//!
//! A handler is done once its last byte sits in the send buffer; up to the
//! connection's send window (64 MiB on larger devices, `keepalive.rs`
//! `transport_windows`) may still be on its way. At 5 Mbit/s upload that is
//! about 107 s, longer than one `PEER_OP_TIMEOUT` (60 s). A fixed wait
//! therefore closed the connection with "reply acknowledgment timeout"
//! (`0x5348`) in the middle of a large download from a slow peer, although
//! the receiver was acknowledging all along. The wait now restarts while the
//! connection still carries stream frames; only a connection without any
//! stream progress for a whole `PEER_OP_TIMEOUT` counts as unacknowledged.
use std::future::Future;

use iroh::endpoint::{Connection, ConnectionStats};

use super::io_deadline::PEER_OP_TIMEOUT;

/// Whether the peer acknowledged the reply that `stopped` waits for.
pub(super) async fn acknowledged<T>(
    connection: &Connection,
    stopped: impl Future<Output = T>,
) -> bool {
    tokio::pin!(stopped);
    let mut seen = stream_progress(&connection.stats());
    loop {
        if super::route_protocol::timeout_for(PEER_OP_TIMEOUT, &mut stopped)
            .await
            .is_ok()
        {
            return true;
        }
        let now = stream_progress(&connection.stats());
        if !still_draining(seen, now) {
            return false;
        }
        seen = now;
    }
}

/// Stream frames sent and received and flow-control credit granted on the
/// connection; QUIC keepalive pings and bare acknowledgements leave it
/// unchanged, so a peer that only answers pings does not count as progress.
fn stream_progress(stats: &ConnectionStats) -> u64 {
    [&stats.frame_tx, &stats.frame_rx]
        .iter()
        .map(|frames| {
            frames
                .stream
                .saturating_add(frames.max_stream_data)
                .saturating_add(frames.max_data)
        })
        .fold(0u64, u64::saturating_add)
}

/// Whether a reply whose wait ran out is still moving: the connection sent
/// or received stream data or credit since the last look.
fn still_draining(before: u64, now: u64) -> bool {
    now != before
}
