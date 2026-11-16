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

use iroh::endpoint::{Connection, SendStream, VarInt};

use super::rate_control::PathKind;
use super::wire_binary::{
    write_frame_record, write_gop_header, FrameRecord, GopHeader, FLAG_CONTINUE, FLAG_KEYFRAME,
    FLAG_MORE, RECORD_HEADER_BYTES,
};
use crate::share::node_screen::{enter_interactive, InteractiveGuard, InteractiveKind};

/// Code with which the host gives up a picture stream (stale, send error)
/// and with which the viewer stops a broken one.
pub const STREAM_RESET_CODE: VarInt = VarInt::from_u32(0x5352);
/// Uni streams a viewer allows: one running and one new picture group per
/// kind (`video`, `preview`), C3.
pub const VIEWER_UNI_STREAMS: u32 = 4;

/// Priority of the host's control stream (plan 2026-11-14 E4): above the
/// picture groups (0), so `PONG`, answers and state frames never wait
/// behind picture data; the clock sync feeds the latency measurement.
pub const CONTROL_STREAM_PRIORITY: i32 = 1;

/// Gives the host's control stream `send` its priority (E4).
pub fn prioritize_control(send: &SendStream) {
    let _ = send.set_priority(CONTROL_STREAM_PRIORITY);
}

/// Whether a session streams pictures a user watches: `view` and `control`,
/// not `probe`, `preview` or metadata. Only such a session counts as
/// interactive (plan 2026-11-14 E3).
pub fn picture_session(mode: super::wire_hello::SessionMode, metadata: bool) -> bool {
    use super::wire_hello::SessionMode;
    !metadata && matches!(mode, SessionMode::View | SessionMode::Control)
}

/// The interactive presence of one screen session (E3): held while the
/// session streams the full view, so bulk transfers of this process yield.
/// That is a `view` or `control` session from its start, and a preview while
/// `SET_MODE` has raised it to the full view (the tile of the group view that
/// became the view); back as a preview it gives the presence up.
pub(crate) struct PicturePresence {
    kind: InteractiveKind,
    /// The session can stream pictures at all (no probe, no metadata channel).
    pictures: bool,
    guard: Option<InteractiveGuard>,
}

impl PicturePresence {
    /// A viewer's session as its hello opened it.
    pub(crate) fn viewer(mode: super::wire_hello::SessionMode, metadata: bool) -> Self {
        let pictures = !metadata && mode != super::wire_hello::SessionMode::Probe;
        let mut presence = Self {
            kind: InteractiveKind::ScreenViewer,
            pictures,
            guard: None,
        };
        presence.set_full(picture_session(mode, metadata));
        presence
    }

    /// A host's picture pipeline; `full`: it starts with the full view.
    pub(crate) fn host(full: bool) -> Self {
        let mut presence = Self {
            kind: InteractiveKind::ScreenHost,
            pictures: true,
            guard: None,
        };
        presence.set_full(full);
        presence
    }

    /// The stream is the full view now (`true`) or a preview.
    pub(crate) fn set_full(&mut self, full: bool) {
        if !(full && self.pictures) {
            self.guard = None;
        } else if self.guard.is_none() {
            self.guard = Some(enter_interactive(self.kind));
        }
    }

    /// Whether the session counts as interactive right now.
    #[cfg(test)]
    pub(crate) fn active(&self) -> bool {
        self.guard.is_some()
    }
}

/// Microseconds of the wall clock since 1970.
pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Path kind and round trip (µs) of the connection's selected path, the
/// same reading as the share's `transport_label`: a relay path is `relay`,
/// any other `direct`; without a selected path the relay budget applies.
pub fn path_of(connection: &Connection) -> (PathKind, u64) {
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
pub fn loss_counters(connection: &Connection) -> (u64, u64) {
    let stats = connection.stats();
    (stats.udp_tx.datagrams, stats.lost_packets)
}

/// Production connections pause their active write budget during route discovery.
/// Standalone transports without an enabled Share policy retain their original bounded IO path.
pub async fn write_with_route<T>(
    connection: &Connection,
    budget: std::time::Duration,
    future: impl std::future::Future<Output = io::Result<T>>,
) -> io::Result<T> {
    if connection.application_path_policy().enabled {
        crate::share::route_protocol::run_with_route(connection, budget, None, future).await
    } else {
        tokio::time::timeout(budget, future)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "screen write stalled"))?
    }
}

/// The host's picture stream of one picture group.
pub struct GopWriter {
    send: SendStream,
    header: GopHeader,
    /// The next frame is the first of a continuation (B-36).
    continues: bool,
}

impl GopWriter {
    pub async fn open(connection: &Connection, header: GopHeader) -> io::Result<Self> {
        let mut send = connection.open_uni().await.map_err(io::Error::other)?;
        write_gop_header(&mut send, &header).await?;
        Ok(Self {
            send,
            header,
            continues: false,
        })
    }

    pub fn header(&self) -> &GopHeader {
        &self.header
    }

    /// Marks the stream, opened and still empty, as a continuation: its
    /// first frame (no key frame) carries `FLAG_CONTINUE` (B-36).
    pub fn mark_continuation(&mut self) {
        self.continues = true;
    }

    /// The flag of the next frame besides `FLAG_KEYFRAME`: `FLAG_CONTINUE`
    /// on the first frame of a continuation, once.
    fn first_flag(&mut self, keyframe: bool) -> u8 {
        if std::mem::take(&mut self.continues) && !keyframe {
            FLAG_CONTINUE
        } else {
            0
        }
    }

    /// Writes the records of one frame (all but the last with `FLAG_MORE`)
    /// and returns their bytes; it returns once the last record is handed
    /// to the stream.
    pub async fn write_frame(
        &mut self,
        frame_id: u64,
        capture_ts_us: u64,
        keyframe: bool,
        records: Vec<Vec<u8>>,
    ) -> io::Result<u64> {
        let records = if self
            .header
            .frame_contract
            .is_some_and(|c| !c.hybrid_regions)
            && self.header.codec == super::negotiate::CodecKind::Tiles
        {
            super::wire_binary::repack_tiles(records, self.header.record_limit())?
        } else {
            records
        };
        let budget =
            super::frame_size_caps::PacketBudget::for_contract(self.header.frame_contract)?;
        let total = records.iter().try_fold(0usize, |n, r| {
            n.checked_add(r.len())
                .and_then(|n| n.checked_add(RECORD_HEADER_BYTES))
        });
        if records.len() > budget.records || total.is_none_or(|n| n > budget.encoded_bytes) {
            return Err(super::wire::invalid("frame exceeds derived packet budget"));
        }
        let count = records.len();
        let mut bytes = 0u64;
        let first = self.first_flag(keyframe);
        for (index, payload) in records.into_iter().enumerate() {
            let mut flags = if keyframe { FLAG_KEYFRAME } else { first };
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
    pub async fn write_frame_admitted(
        &mut self,
        frame_id: u64,
        capture_ts_us: u64,
        keyframe: bool,
        mut records: super::capture_records::CapturedRecords,
    ) -> io::Result<u64> {
        let budget =
            super::frame_size_caps::PacketBudget::for_contract(self.header.frame_contract)?;
        let total = records.iter().try_fold(0usize, |n, r| {
            n.checked_add(r.len())
                .and_then(|n| n.checked_add(RECORD_HEADER_BYTES))
        });
        if records.len() > budget.records
            || total.is_none_or(|n| n > budget.encoded_bytes)
            || records
                .iter()
                .any(|record| record.len() > self.header.record_limit())
        {
            return Err(super::wire::invalid("admitted records exceed GOP contract"));
        }
        let count = records.len();
        records.reverse();
        let mut written = 0u64;
        let first = self.first_flag(keyframe);
        for index in 0..count {
            let bytes = records
                .pop()?
                .ok_or_else(|| super::wire::invalid("admitted record disappeared"))?;
            let (payload, lease) = bytes.into_parts();
            let mut flags = if keyframe { FLAG_KEYFRAME } else { first };
            if index + 1 < count {
                flags |= FLAG_MORE;
            }
            written += (payload.len() + RECORD_HEADER_BYTES) as u64;
            self.write_admitted(frame_id, capture_ts_us, flags, payload)
                .await?;
            drop(lease);
        }
        Ok(written)
    }

    /// One record of an admitted frame. The payload goes into the QUIC send
    /// buffer as it is (`write_chunk`, plan bildkette BK3b, copy 16 of the
    /// BK3 table) instead of being copied there; only the 21 bytes of the
    /// header are copied. Its admission ends when the stream took it, as
    /// before: the bytes the stream holds are bounded by the pacer's queue
    /// guard, not by the capture grant.
    async fn write_admitted(
        &mut self,
        frame_id: u64,
        capture_ts_us: u64,
        flags: u8,
        payload: Vec<u8>,
    ) -> io::Result<()> {
        if payload.len() > self.header.record_limit()
            || flags & !super::wire_binary::KNOWN_FLAGS != 0
        {
            return Err(super::wire::invalid(
                "frame record exceeds its limit or has unknown flags",
            ));
        }
        let head = super::wire_binary::record_head(payload.len(), frame_id, capture_ts_us, flags);
        self.send.write_all(&head).await.map_err(io::Error::from)?;
        self.send
            .write_chunk(bytes::Bytes::from(payload))
            .await
            .map_err(io::Error::from)
    }

    /// Gives the stream up: the viewer drops what it has of it.
    pub fn reset(mut self) {
        let _ = self.send.reset(STREAM_RESET_CODE);
    }

    /// Ends the stream after its last frame.
    pub fn finish(mut self) {
        let _ = self.send.finish();
    }
}

#[path = "gop_stream_read.rs"]
mod read;
pub use read::{
    accept_picture_streams, accept_picture_streams_admitted, accept_picture_streams_with_contract,
    accept_picture_streams_with_memory, picture_channel, PictureEvent,
};

#[cfg(test)]
#[path = "gop_stream_bild_tests.rs"]
mod bild_tests;
