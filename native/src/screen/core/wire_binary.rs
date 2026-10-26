//! Picture streams (decision C3 „Bildgruppen-Stream“): the host opens one
//! QUIC uni stream per picture group. It starts with a 20-byte head `SCRV`,
//! version `u8`, `gop_id u64`, kind `u8`, codec `u8`, profile `u8`, width
//! `u16`, height `u16`; then frame records of a 21-byte head `len u32,
//! frame_id u64, capture_ts_us u64, flags u8` and `len` payload bytes, all
//! big-endian. A record carries at most 16 MiB (`MAX_FRAME` of the share
//! framing), a record of the tile codec at most 4 MiB. Every stream starts
//! with a key frame; frame ids rise across the session; the records of one
//! tile frame share its id and all but the last carry `FLAG_MORE`.
use std::io;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::negotiate::{CodecKind, H264Profile};
use super::pixel_convert::FrameSize;
use super::tiles_codec::MAX_TILES_RECORD_BYTES;
use super::wire::{invalid, BeReader};

pub(crate) const GOP_MAGIC: [u8; 4] = *b"SCRV";
pub(crate) const GOP_VERSION: u8 = 1;
pub(crate) const GOP_HEADER_BYTES: usize = 20;
pub(crate) const RECORD_HEADER_BYTES: usize = 21;
/// Largest record payload: `MAX_FRAME` of `share/core/framing.rs`.
pub(crate) const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
/// The frame is a key frame (IDR, or a tile frame with every tile).
pub(crate) const FLAG_KEYFRAME: u8 = 0x01;
/// More records of the same tile frame follow.
pub(crate) const FLAG_MORE: u8 = 0x02;
const KNOWN_FLAGS: u8 = FLAG_KEYFRAME | FLAG_MORE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StreamKind {
    Video,
    /// Tile previews of the group view, acknowledged one by one (C6).
    Preview,
    /// One still image (`SNAPSHOT`, `se screen snapshot`).
    Snapshot,
}

impl StreamKind {
    fn wire_code(self) -> u8 {
        match self {
            Self::Video => 1,
            Self::Preview => 2,
            Self::Snapshot => 3,
        }
    }

    fn from_wire_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Video),
            2 => Some(Self::Preview),
            3 => Some(Self::Snapshot),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GopHeader {
    pub gop_id: u64,
    /// Negotiated v2 reconstruction contract; absent on every legacy stream.
    pub frame_contract: Option<super::frame_budget::FrameContract>,
    /// Layout of the actual captured family canvas, never an anticipated control snapshot.
    pub family_layout_revision: Option<u64>,
    pub kind: StreamKind,
    pub codec: CodecKind,
    /// Set exactly for H.264.
    pub profile: Option<H264Profile>,
    pub size: FrameSize,
}

impl GopHeader {
    fn check(&self) -> io::Result<()> {
        if self
            .family_layout_revision
            .is_some_and(|revision| revision == 0 || self.frame_contract.is_none())
        {
            return Err(invalid(
                "family layout requires a nonzero revision and a v2 picture contract",
            ));
        }
        if let Some(contract) = self.frame_contract {
            contract.validate()?;
            contract.limits.check(self.size)?;
        } else {
            self.size.check_frame()?;
        }
        match (self.codec, self.profile) {
            (CodecKind::H264, Some(_)) | (CodecKind::Tiles, None) => Ok(()),
            _ => Err(invalid("picture stream profile does not match its codec")),
        }
    }

    pub(crate) fn encode(&self) -> io::Result<[u8; GOP_HEADER_BYTES]> {
        self.check()?;
        let mut out = [0u8; GOP_HEADER_BYTES];
        out[..4].copy_from_slice(&GOP_MAGIC);
        out[4] = self.frame_contract.map_or(GOP_VERSION, |c| c.version);
        out[5..13].copy_from_slice(&self.gop_id.to_be_bytes());
        out[13] = self.kind.wire_code();
        out[14] = self.codec.wire_code();
        out[15] = self.profile.map_or(0, H264Profile::wire_code);
        out[16..18].copy_from_slice(&(self.size.width as u16).to_be_bytes());
        out[18..20].copy_from_slice(&(self.size.height as u16).to_be_bytes());
        Ok(out)
    }

    /// Complete legacy header only. A v2 prefix cannot grant allocations.
    pub(crate) fn decode(bytes: &[u8; GOP_HEADER_BYTES]) -> io::Result<Self> {
        let (header, extended) = Self::decode_prefix(bytes)?;
        if extended {
            return Err(invalid("v2 picture header requires its contract extension"));
        }
        Ok(header)
    }

    pub(crate) fn decode_prefix(bytes: &[u8; GOP_HEADER_BYTES]) -> io::Result<(Self, bool)> {
        let mut reader = BeReader::new(bytes);
        if reader.bytes(4)? != GOP_MAGIC {
            return Err(invalid("picture stream does not start with SCRV"));
        }
        let version = reader.u8()?;
        if version != GOP_VERSION && version != super::frame_budget::FRAME_PROTOCOL {
            return Err(invalid("unsupported picture stream version"));
        }
        let gop_id = reader.u64()?;
        let kind = StreamKind::from_wire_code(reader.u8()?)
            .ok_or_else(|| invalid("unknown picture stream kind"))?;
        let codec = CodecKind::from_wire_code(reader.u8()?)
            .ok_or_else(|| invalid("unknown picture stream codec"))?;
        let profile = match reader.u8()? {
            0 => None,
            code => Some(
                H264Profile::from_wire_code(code)
                    .ok_or_else(|| invalid("unknown picture stream profile"))?,
            ),
        };
        let size = FrameSize::new(u32::from(reader.u16()?), u32::from(reader.u16()?));
        let header = Self {
            gop_id,
            family_layout_revision: None,
            frame_contract: None,
            kind,
            codec,
            profile,
            size,
        };
        if version == GOP_VERSION {
            header.check()?;
        } else {
            if size.width == 0
                || size.height == 0
                || size.width > super::frame_budget::MAX_RESOURCE_SIDE
                || size.height > super::frame_budget::MAX_RESOURCE_SIDE
            {
                return Err(invalid("invalid v2 picture dimensions"));
            }
            if !matches!(
                (codec, profile),
                (CodecKind::H264, Some(_)) | (CodecKind::Tiles, None)
            ) {
                return Err(invalid("picture stream profile does not match its codec"));
            }
        }
        Ok((header, version == super::frame_budget::FRAME_PROTOCOL))
    }

    /// Largest record payload of this stream's codec.
    pub(crate) fn record_limit(&self) -> usize {
        let codec = if self.frame_contract.is_some_and(|c| c.hybrid_regions) {
            MAX_RECORD_BYTES
        } else {
            match self.codec {
                CodecKind::H264 => MAX_RECORD_BYTES,
                CodecKind::Tiles => MAX_TILES_RECORD_BYTES,
            }
        };
        self.frame_contract.map_or(codec, |contract| {
            super::frame_size_caps::PacketBudget::for_contract(Some(contract))
                .map_or(0, |budget| codec.min(budget.record_bytes))
        })
    }
}

/// One frame record as read from a picture stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameRecord {
    pub frame_id: u64,
    /// Host clock when the frame was captured.
    pub capture_ts_us: u64,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl FrameRecord {
    pub(crate) fn is_keyframe(&self) -> bool {
        self.flags & FLAG_KEYFRAME != 0
    }

    pub(crate) fn has_more(&self) -> bool {
        self.flags & FLAG_MORE != 0
    }
}

/// Order rules of the records of one picture stream: the first frame is a
/// key frame; frame ids rise; the records of a tile frame keep its id and
/// flags, and only tile frames have more than one record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecordSequence {
    last_frame: Option<u64>,
    open: Option<(u64, u8)>,
}

impl RecordSequence {
    pub(crate) fn accept(&mut self, header: &GopHeader, record: &FrameRecord) -> io::Result<()> {
        if record.flags & !KNOWN_FLAGS != 0 {
            return Err(invalid("frame record has unknown flags"));
        }
        if record.has_more()
            && header.codec != CodecKind::Tiles
            && !header.frame_contract.is_some_and(|c| c.hybrid_regions)
        {
            return Err(invalid("only tile frames span several records"));
        }
        let base_flags = record.flags & !FLAG_MORE;
        match self.open {
            Some((frame_id, flags)) if frame_id == record.frame_id && flags == base_flags => {}
            Some(_) => return Err(invalid("frame record interrupts an unfinished tile frame")),
            None => {
                let rises = self.last_frame.is_none_or(|last| record.frame_id > last);
                if !rises {
                    return Err(invalid("frame ids of a picture stream must rise"));
                }
                if self.last_frame.is_none() && !record.is_keyframe() {
                    return Err(invalid("a picture stream must start with a key frame"));
                }
                self.last_frame = Some(record.frame_id);
            }
        }
        self.open = record.has_more().then_some((record.frame_id, base_flags));
        Ok(())
    }

    /// Whether the stream ended within a tile frame.
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

#[derive(Serialize, Deserialize)]
struct GopExtension {
    #[serde(flatten)]
    contract: super::frame_budget::FrameContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    family_layout_revision: Option<u64>,
}

pub(crate) async fn write_gop_header<W: AsyncWrite + Unpin>(
    writer: &mut W,
    header: &GopHeader,
) -> io::Result<()> {
    writer.write_all(&header.encode()?).await?;
    if let Some(contract) = header.frame_contract {
        let extension = GopExtension {
            contract,
            family_layout_revision: header.family_layout_revision,
        };
        let json = serde_json::to_vec(&extension).map_err(io::Error::other)?;
        if json.len() > 1024 {
            return Err(invalid("screen contract exceeds 1 KiB"));
        }
        writer.write_all(&(json.len() as u16).to_be_bytes()).await?;
        writer.write_all(&json).await?;
    }
    writer.flush().await
}

pub(crate) async fn read_gop_header<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<GopHeader> {
    let mut bytes = [0u8; GOP_HEADER_BYTES];
    reader.read_exact(&mut bytes).await?;
    let (mut header, extended) = GopHeader::decode_prefix(&bytes)?;
    if extended {
        let len = reader.read_u16().await? as usize;
        if len > 1024 {
            return Err(invalid("screen contract exceeds 1 KiB"));
        }
        let mut json = vec![0; len];
        reader.read_exact(&mut json).await?;
        let extension: GopExtension = serde_json::from_slice(&json).map_err(io::Error::other)?;
        header.frame_contract = Some(extension.contract);
        header.family_layout_revision = extension.family_layout_revision;
        header.check()?;
    }
    Ok(header)
}

/// Writes one record; a payload above the codec's limit is refused before
/// anything is written.
pub(crate) async fn write_frame_record<W: AsyncWrite + Unpin>(
    writer: &mut W,
    header: &GopHeader,
    record: &FrameRecord,
) -> io::Result<()> {
    if record.payload.len() > header.record_limit() || record.flags & !KNOWN_FLAGS != 0 {
        return Err(invalid(
            "frame record exceeds its limit or has unknown flags",
        ));
    }
    let mut head = [0u8; RECORD_HEADER_BYTES];
    head[..4].copy_from_slice(&(record.payload.len() as u32).to_be_bytes());
    head[4..12].copy_from_slice(&record.frame_id.to_be_bytes());
    head[12..20].copy_from_slice(&record.capture_ts_us.to_be_bytes());
    head[20] = record.flags;
    writer.write_all(&head).await?;
    writer.write_all(&record.payload).await?;
    writer.flush().await
}

/// Reads the next record; `None` when the stream ended cleanly between two
/// records. The length is checked against the codec's limit before the
/// payload is allocated.
pub(crate) async fn read_frame_record<R: AsyncRead + Unpin>(
    reader: &mut R,
    header: &GopHeader,
) -> io::Result<Option<FrameRecord>> {
    let mut budget = FrameReadBudget::default();
    read_frame_record_budgeted(reader, header, &mut budget).await
}

#[derive(Default)]
pub(crate) struct FrameReadBudget {
    frame: Option<u64>,
    records: usize,
    bytes: usize,
}

pub(crate) async fn read_frame_record_budgeted<R: AsyncRead + Unpin>(
    reader: &mut R,
    header: &GopHeader,
    budget: &mut FrameReadBudget,
) -> io::Result<Option<FrameRecord>> {
    let bound = super::frame_size_caps::PacketBudget::for_contract(header.frame_contract)?;
    read_frame_record_admitted(reader, header, budget, bound).await
}
pub(crate) async fn read_frame_record_admitted<R: AsyncRead + Unpin>(
    reader: &mut R,
    header: &GopHeader,
    budget: &mut FrameReadBudget,
    bound: super::frame_size_caps::PacketBudget,
) -> io::Result<Option<FrameRecord>> {
    let mut head = [0u8; RECORD_HEADER_BYTES];
    if reader.read(&mut head[..1]).await? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut head[1..]).await?;
    let mut fields = BeReader::new(&head);
    let len = fields.u32()? as usize;
    let frame_id = fields.u64()?;
    let capture_ts_us = fields.u64()?;
    let flags = fields.u8()?;
    if len > header.record_limit().min(bound.record_bytes) {
        return Err(invalid("frame record exceeds the limit of its codec"));
    }
    if budget.frame != Some(frame_id) {
        budget.frame = Some(frame_id);
        budget.records = 0;
        budget.bytes = 0;
    }
    budget.records = budget.records.saturating_add(1);
    budget.bytes = budget
        .bytes
        .saturating_add(RECORD_HEADER_BYTES)
        .saturating_add(len);
    if budget.records > bound.records || budget.bytes > bound.encoded_bytes {
        return Err(invalid(
            "frame transaction exceeds negotiated packet budget",
        ));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(Some(FrameRecord {
        frame_id,
        capture_ts_us,
        flags,
        payload,
    }))
}

/// Repack lossless records at whole tile boundaries; pixels and frame identity
/// are unchanged. A small v2 budget must not make a legal full tile frame stall.
pub(crate) fn repack_tiles(records: Vec<Vec<u8>>, limit: usize) -> io::Result<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut packet = Vec::with_capacity(limit);
    packet.extend_from_slice(&[0, 0]);
    let mut count = 0u16;
    for record in records {
        if record.len() < 2 {
            return Err(invalid("truncated tile record"));
        }
        let tiles = u16::from_be_bytes([record[0], record[1]]);
        let mut at = 2usize;
        for _ in 0..tiles {
            let head = record
                .get(at..at.saturating_add(11))
                .ok_or_else(|| invalid("truncated tile header"))?;
            let len = u32::from_be_bytes([head[7], head[8], head[9], head[10]]) as usize;
            let end = at
                .checked_add(11)
                .and_then(|n| n.checked_add(len))
                .filter(|n| *n <= record.len())
                .ok_or_else(|| invalid("truncated tile payload"))?;
            if end - at + 2 > limit {
                return Err(invalid("one tile exceeds negotiated record budget"));
            }
            if packet.len() + end - at > limit || count == u16::MAX {
                packet[..2].copy_from_slice(&count.to_be_bytes());
                out.push(packet);
                packet = Vec::with_capacity(limit);
                packet.extend_from_slice(&[0, 0]);
                count = 0;
            }
            packet.extend_from_slice(&record[at..end]);
            count += 1;
            at = end;
        }
        if at != record.len() {
            return Err(invalid("trailing tile record bytes"));
        }
    }
    if count > 0 {
        packet[..2].copy_from_slice(&count.to_be_bytes());
        out.push(packet);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "wire_binary_tests.rs"]
mod tests;
