//! Picture streams over QUIC uni streams (decision C3 „Bildgruppen-Stream“,
//! NR N1): the host opens one stream per picture group and writes its head
//! and frame records; the viewer accepts at most `VIEWER_UNI_STREAMS` of
//! them, acknowledges each frame right after its last record (`FRAME_ACK`,
//! `PREVIEW_ACK` with the arrival time) and stops a stream at once when it
//! breaks (reset by the host, a record out of order), because only then its
//! slot comes free. Times are microseconds of the wall clock since 1970 on
//! both sides; `PING`/`PONG` correct the offset (C9).
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
use tokio::sync::mpsc;

use super::rate_control::PathKind;
use super::wire_binary::{
    read_frame_record, read_gop_header, write_frame_record, write_gop_header, FrameRecord,
    GopHeader, RecordSequence, StreamKind, FLAG_KEYFRAME, FLAG_MORE, RECORD_HEADER_BYTES,
};
use super::wire_control::{FrameAck, ViewerFrame};

/// Code with which the host gives up a picture stream (stale, send error)
/// and with which the viewer stops a broken one.
pub(crate) const STREAM_RESET_CODE: VarInt = VarInt::from_u32(0x5352);
/// Uni streams a viewer allows: one running and one new picture group per
/// kind (`video`, `preview`), C3.
pub(crate) const VIEWER_UNI_STREAMS: u32 = 4;
/// Picture events a reader may queue ahead of the decoder; a slower
/// decoder makes the reader wait, so QUIC flow control throttles the host.
const PICTURE_QUEUE: usize = 4;

/// Microseconds of the wall clock since 1970.
pub(crate) fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Path kind and round trip (µs) of the connection's selected path, the
/// same reading as the share's `transport_label`: a relay path is `relay`,
/// any other `direct`; without a selected path the relay budget applies.
pub(crate) fn path_of(connection: &Connection) -> (PathKind, u64) {
    let paths = connection.paths();
    let selected = paths.iter().find(|path| path.is_selected());
    match selected {
        Some(path) => {
            let kind = if path.is_relay() {
                PathKind::Relay
            } else {
                PathKind::Direct
            };
            let rtt = u64::try_from(path.rtt().as_micros()).unwrap_or(u64::MAX);
            (kind, rtt)
        }
        None => (PathKind::Relay, 0),
    }
}

/// Cumulative sent and lost packets of the connection (`Connection::stats`),
/// read once a second for the loss controller.
pub(crate) fn loss_counters(connection: &Connection) -> (u64, u64) {
    let stats = connection.stats();
    (stats.udp_tx.datagrams, stats.lost_packets)
}

/// The host's picture stream of one picture group.
pub(crate) struct GopWriter {
    send: SendStream,
    header: GopHeader,
}

impl GopWriter {
    pub(crate) async fn open(connection: &Connection, header: GopHeader) -> io::Result<Self> {
        let mut send = connection.open_uni().await.map_err(io::Error::other)?;
        write_gop_header(&mut send, &header).await?;
        Ok(Self { send, header })
    }

    pub(crate) fn header(&self) -> &GopHeader {
        &self.header
    }

    /// Writes the records of one frame (all but the last with `FLAG_MORE`)
    /// and returns their bytes; it returns once the last record is handed
    /// to the stream.
    pub(crate) async fn write_frame(
        &mut self,
        frame_id: u64,
        capture_ts_us: u64,
        keyframe: bool,
        records: Vec<Vec<u8>>,
    ) -> io::Result<u64> {
        let count = records.len();
        let mut bytes = 0u64;
        for (index, payload) in records.into_iter().enumerate() {
            let mut flags = if keyframe { FLAG_KEYFRAME } else { 0 };
            if index + 1 < count {
                flags |= FLAG_MORE;
            }
            bytes += (payload.len() + RECORD_HEADER_BYTES) as u64;
            let record = FrameRecord {
                frame_id,
                capture_ts_us,
                flags,
                payload,
            };
            write_frame_record(&mut self.send, &self.header, &record).await?;
        }
        Ok(bytes)
    }

    /// Gives the stream up: the viewer drops what it has of it.
    pub(crate) fn reset(mut self) {
        let _ = self.send.reset(STREAM_RESET_CODE);
    }

    /// Ends the stream after its last frame.
    pub(crate) fn finish(mut self) {
        let _ = self.send.finish();
    }
}

/// What the viewer's readers deliver.
pub(crate) enum PictureEvent {
    /// One record of a picture group; `received_us` right after it arrived.
    Record {
        header: GopHeader,
        record: FrameRecord,
        received_us: u64,
    },
    /// The stream of `gop_id` broke off; its partial frame is lost.
    Broken { gop_id: u64 },
}

/// The viewer's channel of picture events.
pub(crate) fn picture_channel() -> (mpsc::Sender<PictureEvent>, mpsc::Receiver<PictureEvent>) {
    mpsc::channel(PICTURE_QUEUE)
}

/// Accepts the host's picture streams until the connection ends; each runs
/// in a task of its own. `acks` takes the acknowledgements for the control
/// stream's writer.
pub(crate) async fn accept_picture_streams(
    connection: Connection,
    events: mpsc::Sender<PictureEvent>,
    acks: mpsc::Sender<ViewerFrame>,
) {
    while let Ok(recv) = connection.accept_uni().await {
        tokio::spawn(read_picture_stream(recv, events.clone(), acks.clone()));
    }
}

async fn read_picture_stream(
    mut recv: RecvStream,
    events: mpsc::Sender<PictureEvent>,
    acks: mpsc::Sender<ViewerFrame>,
) {
    let header = match read_gop_header(&mut recv).await {
        Ok(header) => header,
        Err(_) => {
            let _ = recv.stop(STREAM_RESET_CODE);
            return;
        }
    };
    let mut sequence = RecordSequence::default();
    loop {
        let record = match read_frame_record(&mut recv, &header).await {
            Ok(Some(record)) => record,
            Ok(None) if !sequence.is_open() => return,
            Ok(None) | Err(_) => break,
        };
        let received_us = now_us();
        if sequence.accept(&header, &record).is_err() {
            break;
        }
        if !record.has_more() {
            let ack = FrameAck {
                gop_id: header.gop_id,
                frame_id: record.frame_id,
                received_us,
            };
            let frame = match header.kind {
                StreamKind::Video => Some(ViewerFrame::FrameAck(ack)),
                StreamKind::Preview => Some(ViewerFrame::PreviewAck(ack)),
                StreamKind::Snapshot => None,
            };
            if let Some(frame) = frame {
                if acks.send(frame).await.is_err() {
                    break;
                }
            }
        }
        let event = PictureEvent::Record {
            header,
            record,
            received_us,
        };
        if events.send(event).await.is_err() {
            break;
        }
    }
    // A broken stream is stopped at once, so its slot comes free (NR N1).
    let _ = recv.stop(STREAM_RESET_CODE);
    let _ = events
        .send(PictureEvent::Broken {
            gop_id: header.gop_id,
        })
        .await;
}
