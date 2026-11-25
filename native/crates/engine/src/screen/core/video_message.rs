//! One picture as a message of its own (`video_transport.rs`): what a
//! picture stream carries for one picture, preceded by its group's header,
//! so the picture needs no stream around it. The datagram carriers split it
//! into packets (`video_packet.rs`); a carrier of whole records moves it as
//! one record (`write_record(&message)`, `read_record()`), and the viewer
//! hands it on with `video_viewer::MessageDelivery::deliver`.
//!
//! Layout, big-endian: `SCRM`, version `u8` (1), `prev_frame_id u64` (the
//! picture of the same group sent before it, 0 for the group's first), the
//! group header exactly as a picture stream starts (`write_gop_header`: 20
//! bytes, the v2 contract extension after them), then the picture's records
//! as a stream carries them (21-byte head, payload; all but the last with
//! `FLAG_MORE`). The checks are those of the picture stream's reader.
use std::io;

use futures_util::FutureExt;

use super::super::frame_budget::{self, FrameContract, FrameLimits};
use super::super::frame_size_caps::PacketBudget;
use super::super::wire::invalid;
use super::super::wire_binary::{
    read_frame_record_admitted, read_gop_header, record_head, write_gop_header, FrameReadBudget,
    FrameRecord, GopHeader, StreamKind, FLAG_CONTINUE, FLAG_KEYFRAME, FLAG_MORE, GOP_HEADER_BYTES,
    KNOWN_FLAGS, RECORD_HEADER_BYTES,
};

pub const MESSAGE_MAGIC: [u8; 4] = *b"SCRM";
pub const MESSAGE_VERSION: u8 = 1;
/// Magic, version and the link to the picture before.
pub const MESSAGE_HEAD: usize = 13;
/// Bytes of a message beyond its records at most: its head and the group
/// header with the largest contract extension (1 KiB).
pub const MESSAGE_OVERHEAD: usize = MESSAGE_HEAD + GOP_HEADER_BYTES + 2 + 1024;

/// What the host knows of the picture it sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureMeta {
    pub frame_id: u64,
    pub capture_ts_us: u64,
    pub keyframe: bool,
    /// The first picture of a group that continues the chain (B-36).
    pub continuation: bool,
    /// The picture of the same group sent before it; 0 for the first.
    pub prev_frame_id: u64,
}

/// One decoded message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureMessage {
    pub header: GopHeader,
    pub prev_frame_id: u64,
    /// At least one record, all of one picture.
    pub records: Vec<FrameRecord>,
}

impl PictureMessage {
    pub fn frame_id(&self) -> u64 {
        self.records.first().map_or(0, |record| record.frame_id)
    }

    pub fn keyframe(&self) -> bool {
        self.records.first().is_some_and(FrameRecord::is_keyframe)
    }

    pub fn continuation(&self) -> bool {
        self.records
            .first()
            .is_some_and(FrameRecord::is_continuation)
    }
}

/// The message of one picture of the group `header`.
pub fn encode(header: &GopHeader, meta: PictureMeta, records: &[Vec<u8>]) -> io::Result<Vec<u8>> {
    if meta.keyframe && meta.continuation {
        return Err(invalid("a key frame is no continuation"));
    }
    let budget = PacketBudget::for_contract(header.frame_contract)?;
    let total = records.iter().try_fold(0usize, |n, record| {
        n.checked_add(record.len())
            .and_then(|n| n.checked_add(RECORD_HEADER_BYTES))
    });
    if records.is_empty()
        || records.len() > budget.records
        || total.is_none_or(|n| n > budget.encoded_bytes)
        || records
            .iter()
            .any(|record| record.len() > header.record_limit())
    {
        return Err(invalid("picture exceeds its group's packet budget"));
    }
    let flags = if meta.keyframe {
        FLAG_KEYFRAME
    } else if meta.continuation {
        FLAG_CONTINUE
    } else {
        0
    };
    let mut out = Vec::with_capacity(MESSAGE_OVERHEAD + total.unwrap_or(0));
    out.extend_from_slice(&MESSAGE_MAGIC);
    out.push(MESSAGE_VERSION);
    out.extend_from_slice(&meta.prev_frame_id.to_be_bytes());
    // An in-memory writer is never pending.
    write_gop_header(&mut out, header)
        .now_or_never()
        .ok_or_else(|| invalid("picture header write did not finish"))??;
    let count = records.len();
    for (index, payload) in records.iter().enumerate() {
        let more = if index + 1 < count { FLAG_MORE } else { 0 };
        out.extend_from_slice(&record_head(
            payload.len(),
            meta.frame_id,
            meta.capture_ts_us,
            flags | more,
        ));
        out.extend_from_slice(payload);
    }
    Ok(out)
}

/// What the viewer accepted for its pictures: the contract of `HELLO_OK`,
/// its local limits and the bytes of its memory lease.
#[derive(Clone, Copy, Debug, Default)]
pub struct Admission {
    pub contract: Option<FrameContract>,
    pub local: Option<FrameLimits>,
    pub memory_bytes: Option<u64>,
}

impl Admission {
    /// The largest message the viewer assembles.
    pub fn max_message(&self) -> io::Result<usize> {
        let budget = PacketBudget::for_receive(self.contract, self.local)?;
        Ok(budget.encoded_bytes.saturating_add(MESSAGE_OVERHEAD))
    }

    /// The checks of `gop_stream_read::read_picture_stream` for a header.
    fn admits(&self, header: &GopHeader) -> bool {
        let owners = if header.frame_contract.is_some() {
            frame_budget::VERSIONED_WORKING_IMAGES
        } else {
            frame_budget::WORKING_IMAGES
        };
        let exceeds = self
            .memory_bytes
            .is_some_and(|bytes| header.size.pixels().saturating_mul(4 * owners) > bytes);
        frame_budget::contract_matches(header.frame_contract, self.contract)
            && !exceeds
            && self
                .local
                .is_none_or(|limits| limits.check(header.size).is_ok())
    }
}

/// Reads a message; every check of a picture stream applies, and a
/// message carries exactly one `video` picture.
pub fn decode(bytes: &[u8], admission: &Admission) -> io::Result<PictureMessage> {
    if bytes.len() < MESSAGE_HEAD || bytes[..4] != MESSAGE_MAGIC {
        return Err(invalid("picture message does not start with SCRM"));
    }
    if bytes[4] != MESSAGE_VERSION {
        return Err(invalid("unsupported picture message version"));
    }
    let mut prev = [0u8; 8];
    prev.copy_from_slice(&bytes[5..MESSAGE_HEAD]);
    let mut reader = &bytes[MESSAGE_HEAD..];
    let header = read_gop_header(&mut reader)
        .now_or_never()
        .ok_or_else(|| invalid("picture header read did not finish"))??;
    if header.kind != StreamKind::Video || !admission.admits(&header) {
        return Err(invalid("picture message outside the accepted contract"));
    }
    let budget = PacketBudget::for_receive(header.frame_contract, admission.local)?;
    let mut allocation = FrameReadBudget::default();
    let mut records: Vec<FrameRecord> = Vec::new();
    while !reader.is_empty() {
        let record = read_frame_record_admitted(&mut reader, &header, &mut allocation, budget)
            .now_or_never()
            .ok_or_else(|| invalid("picture record read did not finish"))??
            .ok_or_else(|| invalid("picture message ends inside a record"))?;
        records.push(record);
    }
    check_records(&header, &records)?;
    Ok(PictureMessage {
        header,
        prev_frame_id: u64::from_be_bytes(prev),
        records,
    })
}

/// One picture: one frame id, time and flags, `FLAG_MORE` on all records
/// but the last and only where a frame may span records.
fn check_records(header: &GopHeader, records: &[FrameRecord]) -> io::Result<()> {
    let first = records
        .first()
        .ok_or_else(|| invalid("picture message without records"))?;
    let base = first.flags & !FLAG_MORE;
    let multi = header.codec == super::super::negotiate::CodecKind::Tiles
        || header.frame_contract.is_some_and(|c| c.hybrid_regions);
    if base & !KNOWN_FLAGS != 0
        || (first.is_continuation() && (first.is_keyframe() || !header.codec.is_video()))
        || (records.len() > 1 && !multi)
    {
        return Err(invalid("picture message has invalid flags"));
    }
    let count = records.len();
    for (index, record) in records.iter().enumerate() {
        let more = index + 1 < count;
        if record.frame_id != first.frame_id
            || record.capture_ts_us != first.capture_ts_us
            || record.flags & !FLAG_MORE != base
            || record.has_more() != more
        {
            return Err(invalid("picture message mixes pictures"));
        }
    }
    Ok(())
}
