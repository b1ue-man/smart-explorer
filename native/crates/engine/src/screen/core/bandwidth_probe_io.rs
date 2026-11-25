//! The probe stream on the line (`bandwidth_probe.rs`): the host's writer
//! and the viewer's count.
use std::io;
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream, WriteError};
use tokio::sync::mpsc;

use super::super::gop_stream::STREAM_RESET_CODE;
use super::super::wire_control::ViewerFrame;
use super::{
    ProbeHead, ProbeMeter, FILLER, PROBE_HEAD_BYTES, PROBE_STREAM_PRIORITY, READ_GRACE_US,
    WINDOW_US,
};

/// The largest chunk the viewer takes of the probe stream at once; it drops
/// each one (QUIC's receive window bounds what waits).
const READ_CHUNK: usize = 64 * 1024;

/// The probe's send side: reset on every way out (the end of the duration,
/// an error, an aborted task), because a dropped `SendStream` finishes its
/// stream and would send the buffered filler after all (noq 1.3
/// `SendStream::drop`).
struct ProbeSend(SendStream);

impl Drop for ProbeSend {
    fn drop(&mut self) {
        let _ = self.0.reset(STREAM_RESET_CODE);
    }
}

/// Host: opens the probe stream of `head` on `connection`, writes filler as
/// fast as QUIC takes it until the head's duration has passed since the
/// call, and resets the stream. Returns the filler bytes QUIC took.
pub async fn send_probe(connection: Connection, head: ProbeHead) -> io::Result<u64> {
    let deadline = tokio::time::Instant::now() + Duration::from_micros(head.duration_us());
    let send = tokio::time::timeout_at(deadline, connection.open_uni())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "probe stream not opened"))?
        .map_err(io::Error::other)?;
    let mut send = ProbeSend(send);
    let _ = send.0.set_priority(PROBE_STREAM_PRIORITY);
    let mut written = 0u64;
    // The end of the duration ends the writing; the reset follows on drop.
    let _ = tokio::time::timeout_at(deadline, fill(&mut send.0, &head, &mut written)).await;
    Ok(written)
}

/// Writes the head and then filler until the stream fails.
async fn fill(
    send: &mut SendStream,
    head: &ProbeHead,
    written: &mut u64,
) -> Result<(), WriteError> {
    send.write_all(&head.encode()).await?;
    loop {
        send.write_chunk(bytes::Bytes::from_static(&FILLER)).await?;
        *written = written.saturating_add(FILLER.len() as u64);
    }
}

/// The bytes that reached `connection` so far, on every path.
fn received(connection: &Connection) -> u64 {
    connection.stats().udp_rx.bytes
}

/// Viewer: counts the probe stream `recv`, whose magic was read already,
/// until the host resets it (or it ends or fails), at most the head's
/// duration plus `READ_GRACE_US`; then stops it and sends `PROBE_RESULT`
/// through `acks` (the control stream's writer). A head that does not parse
/// stops the stream without an answer.
pub async fn read_probe(
    mut recv: RecvStream,
    connection: &Connection,
    acks: &mpsc::Sender<ViewerFrame>,
) {
    let mut rest = [0u8; PROBE_HEAD_BYTES - 4];
    let head = match recv.read_exact(&mut rest).await {
        Ok(()) => ProbeHead::decode_after_magic(&rest).ok(),
        Err(_) => None,
    };
    let Some(head) = head else {
        let _ = recv.stop(STREAM_RESET_CODE);
        return;
    };
    // A monotonic clock: a step of the wall clock never makes a window.
    let started = std::time::Instant::now();
    let now_us = || u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let mut meter = ProbeMeter::new(0, received(connection));
    let mut windows = tokio::time::interval(Duration::from_micros(WINDOW_US));
    windows.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick of an interval completes at once.
    windows.tick().await;
    let limit = tokio::time::sleep(Duration::from_micros(
        head.duration_us().saturating_add(READ_GRACE_US),
    ));
    tokio::pin!(limit);
    loop {
        tokio::select! {
            chunk = recv.read_chunk(READ_CHUNK) => {
                if !matches!(chunk, Ok(Some(_))) {
                    break;
                }
            }
            _ = windows.tick() => meter.sample(now_us(), received(connection)),
            _ = &mut limit => break,
        }
    }
    let _ = recv.stop(STREAM_RESET_CODE);
    meter.sample(now_us(), received(connection));
    let _ = acks
        .send(ViewerFrame::ProbeResult(meter.result(head.probe_id)))
        .await;
}
